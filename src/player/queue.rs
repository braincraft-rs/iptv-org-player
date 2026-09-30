//! The streams of the channel being watched, with fallback to the next one
//! when a stream fails.

use crate::model::Stream;

#[derive(Debug, Clone)]
pub struct StreamQueue {
    streams: Vec<Stream>,
    index: usize,
    /// Every stream has been tried and failed.
    exhausted: bool,
}

impl StreamQueue {
    pub fn new(streams: Vec<Stream>) -> Self {
        Self {
            streams,
            index: 0,
            exhausted: false,
        }
    }

    pub fn streams(&self) -> &[Stream] {
        &self.streams
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn len(&self) -> usize {
        self.streams.len()
    }

    pub fn current(&self) -> Option<&Stream> {
        self.streams.get(self.index)
    }

    pub fn is_exhausted(&self) -> bool {
        self.exhausted
    }

    /// Current stream failed: move to the next one, or mark exhausted.
    pub fn advance(&mut self) -> Option<&Stream> {
        if self.index + 1 < self.streams.len() {
            self.index += 1;
            self.current()
        } else {
            self.exhausted = true;
            None
        }
    }

    /// User picked a specific stream.
    pub fn select(&mut self, index: usize) -> Option<&Stream> {
        if index < self.streams.len() {
            self.index = index;
            self.exhausted = false;
        }
        self.current()
    }

    /// Start over from the best stream.
    pub fn restart(&mut self) -> Option<&Stream> {
        self.select(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue(n: usize) -> StreamQueue {
        StreamQueue::new(
            (0..n)
                .map(|i| Stream {
                    channel_id: "c".into(),
                    url: format!("u{i}"),
                    quality: None,
                    referrer: None,
                    user_agent: None,
                })
                .collect(),
        )
    }

    #[test]
    fn advances_then_exhausts() {
        let mut q = queue(2);
        assert_eq!(q.current().unwrap().url, "u0");
        assert_eq!(q.advance().unwrap().url, "u1");
        assert!(!q.is_exhausted());
        assert!(q.advance().is_none());
        assert!(q.is_exhausted());
    }

    #[test]
    fn select_and_restart_clear_exhausted() {
        let mut q = queue(2);
        q.advance();
        q.advance();
        assert!(q.is_exhausted());
        assert_eq!(q.select(1).unwrap().url, "u1");
        assert!(!q.is_exhausted());
        assert_eq!(q.restart().unwrap().url, "u0");
    }

    #[test]
    fn empty_queue_has_no_current() {
        let mut q = queue(0);
        assert!(q.current().is_none());
        assert!(q.advance().is_none());
        assert!(q.is_exhausted());
    }
}
