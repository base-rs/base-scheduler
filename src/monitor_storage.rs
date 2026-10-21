// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::MonitorState;
use std::{ops::Deref, ptr::NonNull, sync::Arc};

/// Stable, retained storage for a monitor, either allocated separately or embedded in a shared application object.
///
/// The projection is evaluated once. The retained `Arc` prevents exclusive access to or destruction of the containing allocation while any storage owner exists. Backends retain storage independently of application handles and operation futures until their registrations retire. No pinning or unsafe caller code is required.
pub struct MonitorStorage<State: MonitorState> {
    owner: Arc<dyn Send + Sync>,
    state: NonNull<State>,
    application: bool,
}

impl<State: MonitorState> MonitorStorage<State> {
    /// Allocate a monitor state once.
    pub fn new(state: State) -> Self {
        Self::from_arc(Arc::new(state), |state| state)
    }

    /// Retain a monitor embedded in a caller-owned allocation, without another allocation. A projection must return a reference borrowed from its input or longer-lived storage.
    pub fn from_arc<Owner: Send + Sync + 'static>(
        owner: Arc<Owner>,
        project: fn(&Owner) -> &State,
    ) -> Self {
        let projected = project(&owner);
        projected.acquire();
        let state = NonNull::from(projected);
        Self {
            owner,
            state,
            application: true,
        }
    }

    /// Retain storage for a backend without retaining an application handle. The backend must retire its registration when the final application handle is released, then release this owner.
    pub fn retained(&self) -> Self {
        Self {
            owner: Arc::clone(&self.owner),
            state: self.state,
            application: false,
        }
    }

    /// Retain a projected part of this state for a backend, without allocating or acquiring another application handle. The containing owner stays alive until the projected storage is released.
    pub fn retained_project<Projected: MonitorState>(
        &self,
        project: fn(&State) -> &Projected,
    ) -> MonitorStorage<Projected> {
        MonitorStorage {
            owner: Arc::clone(&self.owner),
            state: NonNull::from(project(self)),
            application: false,
        }
    }

    /// Stable address of the state. Dereferencing it after the final owner is released is invalid.
    pub fn as_ptr(&self) -> *const State {
        self.state.as_ptr()
    }
}

impl<State: MonitorState> Clone for MonitorStorage<State> {
    fn clone(&self) -> Self {
        if self.application {
            self.acquire();
        }
        Self {
            owner: Arc::clone(&self.owner),
            state: self.state,
            application: self.application,
        }
    }
}

impl<State: MonitorState> Deref for MonitorStorage<State> {
    type Target = State;
    fn deref(&self) -> &State {
        // SAFETY: The owner retains the allocation from which this shared reference was projected. Safe code cannot move or replace it through a shared reference; no mutable state reference is exposed.
        unsafe { self.state.as_ref() }
    }
}

// SAFETY: Only shared access to a Send + Sync state is exposed, and every copy retains its Send + Sync owner.
unsafe impl<State: MonitorState> Send for MonitorStorage<State> {}
// SAFETY: The same retained, shared-access contract applies to concurrent references.
unsafe impl<State: MonitorState> Sync for MonitorStorage<State> {}

impl<State: MonitorState> Drop for MonitorStorage<State> {
    fn drop(&mut self) {
        if self.application {
            self.release();
        }
    }
}
