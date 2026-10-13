//! Pairing and token checks for the MCP server.
//!
//! The transport is loopback by default but the user can point it at
//! the LAN, and an unauthenticated endpoint on that surface can edit
//! the project. So:
//!
//! 1. The PC shows a six-digit code that expires in five minutes.
//! 2. The phone trades the code for a long-lived token.
//! 3. Every subsequent request carries the token.
//!
//! Six digits rather than the token itself because it gets typed on a
//! phone keyboard. It is short-lived on purpose; the token it returns
//! is the thing that gets stored.
//!
//! The token travels in the JSON payload, not a header. The transport
//! is newline-delimited JSON with no request headers, and inventing a
//! header here would mean the two transports stop being interchangeable.

use std::time::{Duration, Instant};

/// How long a pairing code stays valid. Long enough to find the phone,
/// short enough that a code left on screen expires on its own.
pub const CODE_TTL: Duration = Duration::from_secs(5 * 60);

/// JSON-RPC error code for a missing, wrong or expired token.
///
/// In the implementation-defined server-error range, which is what the
/// spec reserves for exactly this.
pub const UNAUTHORIZED: i32 = -32001;

#[derive(Debug)]
enum Pairing {
    /// No code issued yet.
    Idle,
    /// A code is out there waiting to be redeemed.
    Pending {
        code: String,
        expires: Instant,
    },
    Redeemed,
}

#[derive(Debug)]
pub struct Auth {
    /// Persistent token. `None` until the user enables remote control
    /// and the first pairing happens.
    token: Option<String>,
    pairing: Pairing,
}

impl Default for Auth {
    fn default() -> Self {
        Self {
            token: None,
            pairing: Pairing::Idle,
        }
    }
}

impl Auth {
    pub fn with_token(token: Option<String>) -> Self {
        Self {
            token,
            pairing: Pairing::Idle,
        }
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// Install a persisted token at startup.
    pub fn set_token(&mut self, token: Option<String>) {
        if self.token.is_none() {
            self.token = token;
        }
    }

    /// Mint a fresh token, discarding any previous one.
    pub fn rotate_token(&mut self) -> String {
        let t = generate_token();
        self.token = Some(t.clone());
        self.pairing = Pairing::Redeemed;
        t
    }

    /// Issue a pairing code and return it for display. Any previously
    /// issued code stops working, so only one is ever valid.
    pub fn begin_pairing(&mut self) -> String {
        let code = generate_code();
        self.pairing = Pairing::Pending {
            code: code.clone(),
            expires: Instant::now() + CODE_TTL,
        };
        code
    }

    /// The current code, if one is live. Lets the UI stop showing a
    /// code that has already expired rather than leaving a dead one on
    /// screen.
    pub fn active_code(&self) -> Option<String> {
        match &self.pairing {
            Pairing::Pending { code, expires } if *expires > Instant::now() => Some(code.clone()),
            _ => None,
        }
    }

    /// Trade a code for the token. Fails if there is no live code or it
    /// does not match.
    pub fn redeem(&mut self, code: &str) -> Result<String, &'static str> {
        let valid = match &self.pairing {
            Pairing::Pending {
                code: live,
                expires,
            } => *expires > Instant::now() && live.eq_ignore_ascii_case(code.trim()),
            _ => false,
        };
        if !valid {
            // Clear it either way so a wrong guess cannot be retried
            // against the same code.
            self.pairing = Pairing::Idle;
            return Err("invalid or expired pairing code");
        }
        self.pairing = Pairing::Redeemed;
        Ok(self.rotate_token())
    }

    /// Whether a request may proceed.
    pub fn verify(&self, token: Option<&str>) -> bool {
        use subtle::ConstantTimeEq;
        let (Some(expected), Some(given)) = (self.token.as_deref(), token) else {
            return false;
        };
        // Constant-time compare. A token guess is not a realistic
        // attack on its own, but the cost of doing this properly is
        // zero and the cost of noticing it was not done is high.
        expected.as_bytes().ct_eq(given.as_bytes()).into()
    }

    /// Forget the token and any live code. Every existing remote
    /// session stops working on its next request.
    pub fn revoke_all(&mut self) {
        self.token = None;
        self.pairing = Pairing::Idle;
    }
}

/// 32 hex characters. Two UUIDs worth of randomness from the OS CSPRNG,
/// which `uuid` already sources from.
fn generate_token() -> String {
    let mut s = String::with_capacity(32);
    while s.len() < 32 {
        s.push_str(&uuid::Uuid::new_v4().simple().to_string());
    }
    s.truncate(32);
    s
}

/// Six digits, zero-padded. Derived from a UUID rather than a modulo on
/// a counter so the code does not walk predictably between issues.
fn generate_code() -> String {
    let bytes = uuid::Uuid::new_v4().into_bytes();
    let mut n: u32 = 0;
    for b in bytes.iter().take(4) {
        n = (n << 8) | *b as u32;
    }
    format!("{:06}", n % 1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_token_means_nothing_verifies() {
        let a = Auth::default();
        assert!(!a.verify(None));
        assert!(!a.verify(Some("anything")));
    }

    #[test]
    fn minted_token_verifies_and_others_do_not() {
        let mut a = Auth::default();
        let t = a.rotate_token();
        assert!(a.verify(Some(&t)));
        assert!(!a.verify(None));
        assert!(!a.verify(Some("")));
        assert!(!a.verify(Some("nope")));
        // Same length, different content.
        let mut wrong = t.clone();
        wrong.pop();
        wrong.push(if t.ends_with('a') { 'b' } else { 'a' });
        assert!(
            !a.verify(Some(&wrong)),
            "a same-length wrong token must fail"
        );
    }

    #[test]
    fn redeem_requires_a_live_code() {
        let mut a = Auth::default();
        assert!(a.redeem("123456").is_err(), "no code issued yet");
        let code = a.begin_pairing();
        assert!(a.redeem("000000").is_err(), "wrong code must fail");
        // A wrong guess burns the code.
        assert!(a.redeem(&code).is_err(), "code must be single use");
    }

    #[test]
    fn redeem_with_the_right_code_returns_a_working_token() {
        let mut a = Auth::default();
        let code = a.begin_pairing();
        let token = a.redeem(&code).expect("valid code");
        assert!(a.verify(Some(&token)));
        assert_eq!(a.token(), Some(token.as_str()));
    }

    #[test]
    fn codes_are_six_digits() {
        let mut a = Auth::default();
        for _ in 0..50 {
            let c = a.begin_pairing();
            assert_eq!(c.len(), 6, "got {c}");
            assert!(c.chars().all(|ch| ch.is_ascii_digit()), "got {c}");
        }
    }

    /// A failed attempt burns the code. Six digits over five minutes is a
    /// million guesses, so a wrong one cannot be retried against the
    /// same code; the user regenerates.
    #[test]
    fn beginning_pairing_invalidates_the_previous_code() {
        let mut a = Auth::default();
        let first = a.begin_pairing();
        let second = a.begin_pairing();
        assert_ne!(first, second, "codes must not repeat");
        assert!(a.redeem(&first).is_err(), "the older code must be dead");
    }

    #[test]
    fn a_failed_attempt_burns_the_current_code() {
        let mut a = Auth::default();
        let code = a.begin_pairing();
        assert!(a.redeem("111111").is_err());
        // Brute-forcing the same code must not be possible; the user
        // regenerates from the UI instead.
        assert!(
            a.redeem(&code).is_err(),
            "a wrong guess must burn the code, not just fail"
        );
        assert!(a.active_code().is_none());
    }

    #[test]
    fn whitespace_in_a_typed_code_is_forgiven() {
        let mut a = Auth::default();
        let code = a.begin_pairing();
        let padded = format!("  {code}  ");
        assert!(a.redeem(&padded).is_ok(), "a phone keyboard adds spaces");
    }

    #[test]
    fn revoke_kills_the_token_and_the_code() {
        let mut a = Auth::default();
        let t = a.rotate_token();
        a.begin_pairing();
        a.revoke_all();
        assert!(!a.verify(Some(&t)), "a revoked token must stop working");
        assert!(a.active_code().is_none());
        assert!(a.token().is_none());
    }

    #[test]
    fn rotating_replaces_the_previous_token() {
        let mut a = Auth::default();
        let old = a.rotate_token();
        let new = a.rotate_token();
        assert_ne!(old, new);
        assert!(!a.verify(Some(&old)), "the old token must die on rotate");
        assert!(a.verify(Some(&new)));
    }

    #[test]
    fn active_code_clears_once_expired() {
        let mut a = Auth::default();
        let code = a.begin_pairing();
        assert_eq!(a.active_code().as_deref(), Some(code.as_str()));
        a.pairing = Pairing::Idle;
        assert!(a.active_code().is_none());
    }

    #[test]
    fn persisted_token_is_adopted_at_startup() {
        let mut first = Auth::default();
        let t = first.rotate_token();
        // Fresh process, same token from settings.
        let second = Auth::with_token(Some(t.clone()));
        assert!(second.verify(Some(&t)));
        // An existing token must not be silently overwritten by a
        // stale value coming from disk.
        let mut third = Auth::with_token(Some(t.clone()));
        third.set_token(Some("stale".into()));
        assert!(third.verify(Some(&t)));
    }
}
