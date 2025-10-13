#![allow(dead_code)]

use network_tap_light::{NetworkSnapshot, NetworkSummary};

#[derive(Clone, Debug)]
pub struct PageHealth {
    pub alive: bool,
    pub quiet: bool,
    pub window_ms: u64,
    pub request_count: u64,
    pub responses_2xx: u64,
    pub responses_4xx: u64,
    pub responses_5xx: u64,
    pub inflight: u64,
    pub since_last_activity_ms: u64,
}

impl Default for PageHealth {
    fn default() -> Self {
        Self {
            alive: true,
            quiet: true,
            window_ms: 0,
            request_count: 0,
            responses_2xx: 0,
            responses_4xx: 0,
            responses_5xx: 0,
            inflight: 0,
            since_last_activity_ms: 0,
        }
    }
}

impl PageHealth {
    pub fn update_from_summary(&mut self, summary: &NetworkSummary) {
        self.window_ms = summary.window_ms;
        self.request_count = summary.req;
        self.responses_2xx = summary.res2xx;
        self.responses_4xx = summary.res4xx;
        self.responses_5xx = summary.res5xx;
        self.inflight = summary.inflight;
        self.quiet = summary.quiet;
        self.since_last_activity_ms = summary.since_last_activity_ms;
        self.alive = !summary.quiet || summary.inflight > 0;
    }

    pub fn update_from_snapshot(&mut self, snapshot: &NetworkSnapshot) {
        self.window_ms = snapshot.window_ms;
        self.request_count = snapshot.req;
        self.responses_2xx = snapshot.res2xx;
        self.responses_4xx = snapshot.res4xx;
        self.responses_5xx = snapshot.res5xx;
        self.inflight = snapshot.inflight;
        self.quiet = snapshot.quiet;
        self.since_last_activity_ms = snapshot.since_last_activity_ms;
        self.alive = !snapshot.quiet || snapshot.inflight > 0;
    }
}
