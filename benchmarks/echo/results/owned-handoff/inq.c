// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

// Kernel floor: one-shot RECV versus persistent POLL_ADD followed by recv.
// Build: cc -O3 -g -Wall -Wextra -Werror -pthread owned_recv.c -luring -o owned-recv
// Run with the normal echo client: owned-recv server recv|recv-poll-first|recv-split|poll|poll-short 1 CONNECTIONS
// Every mode has one ring, fixed per-connection buffers and direct nonblocking sends.
// An idle thread gives libc the same multithreaded syscall behavior as Native.
#define _GNU_SOURCE
#include <assert.h>
#include <errno.h>
#include <liburing.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <poll.h>
#include <pthread.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

#define BUFFER_SIZE 65536
#define WRITE_TAG (UINT64_C(1) << 63)

static pthread_mutex_t idle_lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t idle_changed = PTHREAD_COND_INITIALIZER;
static bool finished;

static void *idle_thread(void *unused) {
    (void)unused;
    pthread_mutex_lock(&idle_lock);
    while (!finished) pthread_cond_wait(&idle_changed, &idle_lock);
    pthread_mutex_unlock(&idle_lock);
    return NULL;
}

struct connection {
    int descriptor;
    unsigned char *buffer;
    size_t position, length;
    bool queued, waiting_write, drained;
};

struct reactor {
    struct io_uring ring;
    struct connection *connections;
    unsigned count, remaining;
    bool owned, poll_first, split, short_read, inq;
};

static void fail(const char *operation) {
    perror(operation);
    exit(1);
}

static void receive(struct reactor *reactor, unsigned index) {
    struct connection *connection = &reactor->connections[index];
    struct io_uring_sqe *entry = io_uring_get_sqe(&reactor->ring);
    assert(entry);
    if (reactor->owned) {
        io_uring_prep_recv(entry, connection->descriptor, connection->buffer, BUFFER_SIZE, 0);
        if (reactor->poll_first) entry->ioprio |= IORING_RECVSEND_POLL_FIRST;
    } else {
        io_uring_prep_poll_multishot(entry, connection->descriptor, POLLIN | POLLRDHUP);
    }
    entry->user_data = index;
}

static void process(struct reactor *reactor, unsigned index) {
    struct connection *connection = &reactor->connections[index];
    for (;;) {
        while (connection->position < connection->length) {
            ssize_t count = send(connection->descriptor, connection->buffer + connection->position,
                connection->length - connection->position, MSG_DONTWAIT | MSG_NOSIGNAL);
            if (count > 0) connection->position += count;
            else if (count < 0 && errno == EINTR) continue;
            else if (count < 0 && errno == EAGAIN) {
                assert(!connection->waiting_write);
                connection->waiting_write = true;
                struct io_uring_sqe *entry = io_uring_get_sqe(&reactor->ring);
                assert(entry);
                io_uring_prep_poll_add(entry, connection->descriptor, POLLOUT);
                entry->user_data = WRITE_TAG | index;
                return;
            } else fail("send");
        }
        if (reactor->owned) {
            connection->position = connection->length = 0;
            receive(reactor, index);
            return;
        }
        bool short_read = (reactor->short_read && connection->length && connection->length < BUFFER_SIZE) || (reactor->inq && connection->length && connection->drained);
        connection->position = connection->length = 0;
        // This diagnostic matches Tokio's short-read behavior; it does not model urgent data.
        if (short_read) return;
        ssize_t count;
        if (reactor->inq) {
            struct iovec buffer = {.iov_base = connection->buffer, .iov_len = BUFFER_SIZE};
            union { struct cmsghdr alignment; unsigned char bytes[CMSG_SPACE(sizeof(int))]; } control;
            struct msghdr message = {.msg_iov = &buffer, .msg_iovlen = 1,
                .msg_control = control.bytes, .msg_controllen = sizeof(control.bytes)};
            count = recvmsg(connection->descriptor, &message, MSG_DONTWAIT);
            connection->drained = false;
            if (count > 0 && !(message.msg_flags & MSG_CTRUNC)) {
                for (struct cmsghdr *header = CMSG_FIRSTHDR(&message); header; header = CMSG_NXTHDR(&message, header)) {
                    if (header->cmsg_level == IPPROTO_TCP && header->cmsg_type == TCP_CM_INQ && header->cmsg_len >= CMSG_LEN(sizeof(int))) {
                        int remaining;
                        memcpy(&remaining, CMSG_DATA(header), sizeof(remaining));
                        connection->drained = remaining == 0;
                    }
                }
            }
        } else {
            count = recv(connection->descriptor, connection->buffer, BUFFER_SIZE, MSG_DONTWAIT);
        }
        if (count > 0) connection->length = count;
        else if (count == 0) {
            close(connection->descriptor);
            connection->descriptor = -1;
            reactor->remaining--;
            return;
        } else if (errno == EAGAIN) return;
        else if (errno != EINTR) fail("recv");
    }
}

static void run(struct reactor *reactor) {
    unsigned capacity = reactor->count * 4 + 256;
    unsigned *ready = calloc(reactor->count, sizeof(*ready));
    if (!ready) fail("allocate ready batch");
    struct io_uring_params parameters = {.flags = IORING_SETUP_COOP_TASKRUN | IORING_SETUP_TASKRUN_FLAG};
    int result = io_uring_queue_init_params(capacity, &reactor->ring, &parameters);
    if (result < 0) { errno = -result; fail("ring setup"); }
    for (unsigned index = 0; index < reactor->count; index++) receive(reactor, index);
    while (reactor->remaining) {
        if (reactor->split && io_uring_sq_ready(&reactor->ring)) {
            result = io_uring_submit(&reactor->ring);
            if (result < 0) {
                if (result == -EINTR) continue;
                errno = -result; fail("ring submit");
            }
        }
        // One entry submits the next owned reads and waits, unlike split submit/wait turns.
        result = io_uring_submit_and_wait(&reactor->ring, 1);
        if (result < 0) {
            if (result == -EINTR) continue;
            errno = -result; fail("ring wait");
        }
        unsigned head, consumed = 0, length = 0;
        struct io_uring_cqe *completion;
        io_uring_for_each_cqe(&reactor->ring, head, completion) {
            unsigned index = completion->user_data & ~WRITE_TAG;
            assert(index < reactor->count);
            struct connection *connection = &reactor->connections[index];
            consumed++;
            if (connection->descriptor < 0) continue;
            if (completion->user_data & WRITE_TAG) {
                connection->waiting_write = false;
                if (completion->res < 0) { errno = -completion->res; fail("write readiness"); }
            } else if (reactor->owned) {
                if (completion->res == -EAGAIN || completion->res == -EINTR) {
                    receive(reactor, index);
                    continue;
                }
                if (completion->res < 0) { errno = -completion->res; fail("read completion"); }
                if (!completion->res) {
                    close(connection->descriptor);
                    connection->descriptor = -1;
                    reactor->remaining--;
                    continue;
                }
                assert(connection->position == connection->length);
                connection->position = 0;
                connection->length = completion->res;
            } else {
                if (completion->res < 0) { errno = -completion->res; fail("read readiness"); }
                assert(completion->flags & IORING_CQE_F_MORE);
                // A read-ready CQE can arrive while output is backpressured. The write waiter owns resumption until that buffer has been sent.
                if (connection->waiting_write) continue;
            }
            if (!connection->queued) {
                connection->queued = true;
                ready[length++] = index;
            }
        }
        io_uring_cq_advance(&reactor->ring, consumed);
        for (unsigned position = 0; position < length; position++) {
            unsigned index = ready[position];
            reactor->connections[index].queued = false;
            process(reactor, index);
        }
    }
    io_uring_queue_exit(&reactor->ring);
    free(ready);
}

int main(int argc, char **argv) {
    if (argc != 5 || strcmp(argv[1], "server") || strcmp(argv[3], "1")) return 2;
    unsigned count = strtoul(argv[4], NULL, 10);
    if (!count || count > 4096) return 2;
    bool owned = !strcmp(argv[2], "recv") || !strcmp(argv[2], "recv-poll-first") || !strcmp(argv[2], "recv-split");
    if (!owned && strcmp(argv[2], "poll") && strcmp(argv[2], "poll-short") && strcmp(argv[2], "poll-inq")) return 2;
    int listener = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (listener < 0) fail("listener");
    struct sockaddr_in address = {.sin_family = AF_INET, .sin_addr.s_addr = htonl(INADDR_LOOPBACK)};
    if (bind(listener, (struct sockaddr *)&address, sizeof(address))) fail("bind");
    if (listen(listener, 4096)) fail("listen");
    socklen_t size = sizeof(address);
    if (getsockname(listener, (struct sockaddr *)&address, &size)) fail("listener address");
    printf("127.0.0.1:%u\n", ntohs(address.sin_port));
    fflush(stdout);
    struct connection *connections = calloc(count, sizeof(*connections));
    if (!connections) fail("connections");
    for (unsigned index = 0; index < count; index++) {
        int descriptor = accept4(listener, NULL, NULL, SOCK_NONBLOCK | SOCK_CLOEXEC);
        if (descriptor < 0) fail("accept");
        int enabled = 1;
        if (setsockopt(descriptor, IPPROTO_TCP, TCP_NODELAY, &enabled, sizeof(enabled))) fail("TCP_NODELAY");
        if (!strcmp(argv[2], "poll-inq") && setsockopt(descriptor, IPPROTO_TCP, TCP_INQ, &enabled, sizeof(enabled))) fail("TCP_INQ");
        connections[index].descriptor = descriptor;
        connections[index].buffer = malloc(BUFFER_SIZE);
        if (!connections[index].buffer) fail("buffer");
    }
    close(listener);
    pthread_t idle;
    int result = pthread_create(&idle, NULL, idle_thread, NULL);
    if (result) { errno = result; fail("idle thread"); }
    struct reactor reactor = {.connections = connections, .count = count, .remaining = count,
        .owned = owned, .poll_first = !strcmp(argv[2], "recv-poll-first"),
        .split = !strcmp(argv[2], "recv-split"), .short_read = !strcmp(argv[2], "poll-short"), .inq = !strcmp(argv[2], "poll-inq")};
    run(&reactor);
    for (unsigned index = 0; index < count; index++) free(connections[index].buffer);
    free(connections);
    pthread_mutex_lock(&idle_lock);
    finished = true;
    pthread_cond_signal(&idle_changed);
    pthread_mutex_unlock(&idle_lock);
    pthread_join(idle, NULL);
    return 0;
}
