//! Static, dummy, deterministic — the same rule `silka-dashboard::data` and
//! `silka-account::data` follow: a real-enough shape, generated rather than
//! written out by hand, so the same input always gives the same series and a
//! golden test never drifts.
//!
//! One thing here is not decoration: [`history_len`] and [`message_at`] are
//! written as *pure, index-addressed* functions rather than a `Vec` built up
//! front, because that is what lets [`crate::thread`] load a conversation's
//! **middle** — the last 40 messages, say — without ever materializing the
//! messages before it.

/// One entry in the inbox.
#[derive(Debug, Clone, Copy)]
pub struct Conversation {
    /// Stable id — also the seed [`history_len`]/[`message_at`] key off.
    pub id: usize,
    /// The other person's name.
    pub name: &'static str,
}

/// The people in the inbox, in the order the list shows them.
pub const CONVERSATIONS: [Conversation; 24] = [
    conv(0, "Dian Permata"),
    conv(1, "Bagas Nugroho"),
    conv(2, "Sari Wulandari"),
    conv(3, "Rizky Pratama"),
    conv(4, "Yanto Kurniawan"),
    conv(5, "Dewi Lestari"),
    conv(6, "Hendra Wijaya"),
    conv(7, "Sinta Maharani"),
    conv(8, "Agus Setiawan"),
    conv(9, "Putri Ramadhani"),
    conv(10, "Fajar Nugraha"),
    conv(11, "Indah Permatasari"),
    conv(12, "Wahyu Hidayat"),
    conv(13, "Ayu Kusuma"),
    conv(14, "Doni Saputra"),
    conv(15, "Lestari Handayani"),
    conv(16, "Eko Prasetyo"),
    conv(17, "Maya Anggraini"),
    conv(18, "Budi Santoso"),
    conv(19, "Rina Marlina"),
    conv(20, "Anton Wibowo"),
    conv(21, "Citra Dewanti"),
    conv(22, "Gilang Ramadhan"),
    conv(23, "Novita Sari"),
];

const fn conv(id: usize, name: &'static str) -> Conversation {
    Conversation { id, name }
}

/// One message in a thread.
#[derive(Debug, Clone)]
pub struct Message {
    /// Sent by the signed-in user rather than the other person.
    pub from_me: bool,
    /// The body.
    pub text: String,
    /// How long ago it was sent, in minutes — 0 is the newest message a
    /// conversation has.
    pub minutes_ago: i64,
}

/// A deterministic pseudo-random stream — the same one `silka-dashboard`'s
/// trend chart uses, so the same seed always gives the same series and a
/// golden test never drifts.
fn noise(seed: u64) -> u64 {
    let mut x = seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x
}

/// How many messages `conv`'s history holds, oldest to newest.
///
/// Deliberately uneven (40 to 260) rather than a round number: a thread that
/// runs out of history after exactly one page would never actually exercise
/// the "no more to load" branch [`crate::thread`] has to get right.
pub fn history_len(conv: usize) -> usize {
    40 + (noise(conv as u64 * 97 + 1) % 220) as usize
}

/// A short pool of message bodies, rotated through pseudo-randomly —
/// realistic enough to read as a conversation, not `"Message #41"`.
const BODIES: [&str; 20] = [
    "Hey, are you around?",
    "Sounds good, see you then.",
    "Can you send me the file when you get a chance?",
    "On my way!",
    "Sorry, got caught up — running 10 minutes late.",
    "That works for me.",
    "Let's do Thursday instead.",
    "Just sent it over.",
    "Thanks so much for this.",
    "No worries at all.",
    "What time were you thinking?",
    "I'll take a look tonight.",
    "Perfect, talk soon.",
    "Can we push this to next week?",
    "Got it, appreciate the heads up.",
    "Haha, exactly.",
    "Let me check and get back to you.",
    "Deal.",
    "Miss you, let's catch up soon.",
    "Sent the invite, let me know if it works.",
];

/// The message at `index` (0 = oldest) in `conv`'s history.
///
/// A pure function of `(conv, index)` — the whole reason loading "just the
/// last 40" and later "the 40 before that" never has to touch the messages
/// in between, and never disagrees with itself about what they said.
pub fn message_at(conv: usize, index: usize) -> Message {
    let total = history_len(conv);
    let n = noise(conv as u64 * 104_729 + index as u64 * 7 + 3);
    Message {
        // Not a coin flip: real threads run in bursts of one side talking,
        // which is what makes alternating-every-line data look synthetic.
        from_me: (n / 5) % 3 != 0,
        text: BODIES[(n % BODIES.len() as u64) as usize].to_string(),
        minutes_ago: ((total - 1 - index) as i64) * 7 + (n % 5) as i64,
    }
}

/// A human-readable "how long ago", coarse on purpose — a chat timestamp
/// says "2h ago", not "127 minutes ago".
pub fn relative_time(minutes_ago: i64) -> String {
    match minutes_ago {
        0 => "Just now".to_string(),
        m if m < 60 => format!("{m}m ago"),
        m if m < 60 * 24 => format!("{}h ago", m / 60),
        m => format!("{}d ago", m / (60 * 24)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_conversation_has_a_name_and_a_unique_id() {
        let mut ids: Vec<usize> = CONVERSATIONS.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "two conversations share an id");
        for c in CONVERSATIONS {
            assert!(!c.name.is_empty());
        }
    }

    #[test]
    fn history_length_is_deterministic_and_never_trivially_short() {
        for c in CONVERSATIONS {
            let a = history_len(c.id);
            let b = history_len(c.id);
            assert_eq!(a, b);
            assert!(a >= 40, "conversation {}: only {a} messages", c.id);
        }
    }

    #[test]
    fn message_at_is_a_pure_function_of_its_index() {
        let a = message_at(3, 17);
        let b = message_at(3, 17);
        assert_eq!(a.text, b.text);
        assert_eq!(a.from_me, b.from_me);
        assert_eq!(a.minutes_ago, b.minutes_ago);
    }

    #[test]
    fn the_newest_message_is_the_most_recent_one() {
        // Index `history_len - 1` is defined as "newest", which the thread
        // relies on to show 0 (or close to it) minutes ago at the bottom.
        for c in [CONVERSATIONS[0], CONVERSATIONS[5], CONVERSATIONS[12]] {
            let last = history_len(c.id) - 1;
            let newest = message_at(c.id, last);
            assert!(
                newest.minutes_ago < 10,
                "conversation {}: newest message is {}m old",
                c.id,
                newest.minutes_ago
            );
        }
    }

    #[test]
    fn older_messages_are_further_in_the_past() {
        let total = history_len(2);
        let oldest = message_at(2, 0);
        let newest = message_at(2, total - 1);
        assert!(oldest.minutes_ago > newest.minutes_ago);
    }

    #[test]
    fn relative_time_reads_in_coarse_human_units() {
        assert_eq!(relative_time(0), "Just now");
        assert_eq!(relative_time(5), "5m ago");
        assert_eq!(relative_time(125), "2h ago");
        assert_eq!(relative_time(60 * 24 * 3), "3d ago");
    }
}
