// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::Controller;
use std::sync::Arc;

// Ring-local values always travel with their owning endpoint. Polling-thread changes do not change this owner.
pub(super) struct RingBinding<T> {
    pub controller: Arc<Controller>,
    pub state: T,
}

impl<T> RingBinding<T> {
    pub fn new(controller: &Arc<Controller>, state: T) -> Self {
        Self {
            controller: Arc::clone(controller),
            state,
        }
    }

    pub fn matches(&self, controller: &Arc<Controller>) -> bool {
        Arc::ptr_eq(&self.controller, controller)
    }
}

// Passive registrations can coexist on different rings. Keep the current ring inline; returning to an already visited ring reuses its state without registration syscalls or allocations.
pub(super) struct RingCache<T> {
    current: Option<RingBinding<T>>,
    others: Vec<RingBinding<T>>,
}

impl<T> Default for RingCache<T> {
    fn default() -> Self {
        Self {
            current: None,
            others: Vec::new(),
        }
    }
}

impl<T> RingCache<T> {
    pub fn get_or_insert(
        &mut self,
        controller: &Arc<Controller>,
        create: impl FnOnce() -> T,
    ) -> &T {
        if !self
            .current
            .as_ref()
            .is_some_and(|binding| binding.matches(controller))
        {
            let next = match self
                .others
                .iter()
                .position(|binding| binding.matches(controller))
            {
                Some(index) => self.others.swap_remove(index),
                None => RingBinding::new(controller, create()),
            };
            if let Some(previous) = self.current.replace(next) {
                self.others.push(previous);
            }
        }
        &self.current.as_ref().unwrap().state
    }

    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.current
            .iter()
            .chain(&self.others)
            .map(|binding| &binding.state)
    }
}
