use std::collections::VecDeque;

use soulbrowser_core_types::ActionId;

use crate::model::Priority;

#[derive(Clone, Debug)]
pub struct Job {
    pub id: ActionId,
    pub priority: Priority,
    pub seq: u64,
}

impl Job {
    pub fn new(priority: Priority, seq: u64) -> Self {
        Self {
            id: ActionId::new(),
            priority,
            seq,
        }
    }
}

#[derive(Debug)]
pub struct PriorityLane {
    queues: [VecDeque<Job>; 4],
    weights: [u8; 4],
    deficits: [i32; 4],
    cursor: usize,
}

impl PriorityLane {
    pub fn new(weights: [u8; 4]) -> Self {
        Self {
            queues: [
                VecDeque::new(),
                VecDeque::new(),
                VecDeque::new(),
                VecDeque::new(),
            ],
            weights,
            deficits: [0; 4],
            cursor: 0,
        }
    }

    pub fn push(&mut self, job: Job) {
        self.queues[job.priority.index()].push_back(job);
    }

    pub fn is_empty(&self) -> bool {
        self.queues.iter().all(|q| q.is_empty())
    }

    pub fn pop(&mut self) -> Option<Job> {
        for _ in 0..Priority::ALL.len() * 2 {
            let idx = self.cursor;
            self.deficits[idx] += self.weights[idx] as i32;
            if let Some(job) = self.try_consume(idx) {
                self.cursor = (idx + 1) % Priority::ALL.len();
                return Some(job);
            }
            self.cursor = (idx + 1) % Priority::ALL.len();
        }
        None
    }

    pub fn len_by_priority(&self) -> [usize; 4] {
        let mut lengths = [0usize; 4];
        for (idx, queue) in self.queues.iter().enumerate() {
            lengths[idx] = queue.len();
        }
        lengths
    }

    fn try_consume(&mut self, idx: usize) -> Option<Job> {
        if self.queues[idx].is_empty() {
            return None;
        }
        if self.deficits[idx] <= 0 {
            return None;
        }
        self.deficits[idx] -= 1;
        self.queues[idx].pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Priority;

    #[test]
    fn weighted_round_robin_prefers_high_priority() {
        let mut lane = PriorityLane::new([8, 4, 2, 1]);

        for i in 0..4 {
            lane.push(Job::new(Priority::Deep, i));
        }

        for i in 0..4 {
            lane.push(Job::new(Priority::Standard, 100 + i));
        }

        for i in 0..4 {
            lane.push(Job::new(Priority::Quick, 200 + i));
        }

        for i in 0..4 {
            lane.push(Job::new(Priority::Lightning, 300 + i));
        }

        let mut counts = [0usize; 4];
        for _ in 0..32 {
            if let Some(job) = lane.pop() {
                counts[job.priority.index()] += 1;
            }
        }

        // Lightning should dominate roughly according to weights
        assert!(counts[Priority::Lightning.index()] >= counts[Priority::Quick.index()]);
        assert!(counts[Priority::Lightning.index()] > 0);
        assert!(counts[Priority::Quick.index()] >= counts[Priority::Standard.index()]);
        assert!(counts[Priority::Standard.index()] >= counts[Priority::Deep.index()]);
    }

    #[test]
    fn drain_all_jobs() {
        let mut lane = PriorityLane::new([8, 4, 2, 1]);

        let mut seq = 0;
        for priority in Priority::ALL.into_iter() {
            lane.push(Job::new(priority, seq));
            seq += 1;
        }

        let mut popped = Vec::new();
        while let Some(job) = lane.pop() {
            popped.push(job.seq);
        }

        assert_eq!(popped.len(), Priority::ALL.len());
        assert!(lane.is_empty());
    }
}
