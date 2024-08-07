// SPDX-License-Identifier: MPL-2.0

//! Tasks are the unit of code execution.

mod priority;
mod processor;
mod scheduler;
#[allow(clippy::module_inception)]
mod task;

pub use self::{
    priority::Priority,
    processor::{disable_preempt, preempt, schedule, DisablePreemptGuard},
    scheduler::{add_task, set_scheduler, FifoScheduler, Scheduler},
    task::{Task, TaskAdapter, TaskContextApi, TaskOptions, TaskStatus},
};
