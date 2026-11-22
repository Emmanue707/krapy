use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    next: Duration,
}

impl Backoff {
    const MIN: Duration = Duration::from_millis(250);
    const MAX: Duration = Duration::from_millis(4000);

    pub fn new() -> Self {
        Self { next: Self::MIN }
    }

    pub fn delay(&self) -> Duration {
        self.next
    }

    pub fn advance(&mut self) {
        self.next = self.next.saturating_mul(2).min(Self::MAX);
    }

    pub fn reset(&mut self) {
        self.next = Self::MIN;
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_250ms() {
        let b = Backoff::new();
        assert_eq!(b.delay(), Duration::from_millis(250));
    }

    #[test]
    fn doubles_to_cap() {
        let mut b = Backoff::new();
        for _ in 0..5 {
            b.advance();
        }
        assert_eq!(b.delay(), Duration::from_millis(4000));
        b.advance();
        assert_eq!(b.delay(), Duration::from_millis(4000));
    }

    #[test]
    fn reset() {
        let mut b = Backoff::new();
        b.advance();
        b.advance();
        b.reset();
        assert_eq!(b.delay(), Duration::from_millis(250));
    }
}
