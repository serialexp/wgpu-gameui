//! The texts the tree keeps shaped (`docs/design/prepared-text.md`, Phase 4).
//!
//! A text node keeps its [`PreparedText`](wgpu_gameui::PreparedText) from
//! its first measure, so that measuring it at the width it is offered and
//! laying it out at the width it is given shape it once, and a frame that
//! measures it again shapes nothing. The tree keeps a budget of their bytes:
//! past it, the shaping of the texts laid out longest ago is dropped (each
//! keeps the layout it is drawn with), and one measured again is prepared
//! again. The texts are chained from the most recently laid out to the
//! least, so keeping to the budget costs a step per text dropped.

use crate::ui::{NO_PARENT, Ui};

/// The default budget: 64 MB, room for about 8,500 kept labels
/// (`docs/design/prepared-text.md`, Phase 0's numbers).
pub(crate) const TEXT_BUDGET: u64 = 64 << 20;

/// The chain of text nodes holding their shaping, and what they hold.
pub(crate) struct Texts {
    /// The most recently laid out, or `NO_PARENT`.
    newest: u32,
    /// The least recently laid out, or `NO_PARENT`.
    oldest: u32,
    /// How many are chained.
    pub(crate) count: usize,
    /// The bytes their shaping holds, as each last counted it.
    pub(crate) bytes: u64,
    pub(crate) budget: u64,
    /// Shaping dropped to keep to the budget, ever.
    pub(crate) dropped: u64,
}

impl Default for Texts {
    fn default() -> Self {
        Self {
            newest: NO_PARENT,
            oldest: NO_PARENT,
            count: 0,
            bytes: 0,
            budget: TEXT_BUDGET,
            dropped: 0,
        }
    }
}

/// A text node's place in the chain.
#[derive(Clone, Copy)]
pub(crate) struct Link {
    /// Laid out more recently, or `NO_PARENT`.
    newer: u32,
    /// Laid out less recently, or `NO_PARENT`.
    older: u32,
    /// It is in the chain.
    chained: bool,
    /// Its shaping's bytes when it was last counted.
    bytes: u64,
}

impl Default for Link {
    fn default() -> Self {
        Self {
            newer: NO_PARENT,
            older: NO_PARENT,
            chained: false,
            bytes: 0,
        }
    }
}

impl Ui {
    fn link_of(&mut self, n: u32) -> &mut Link {
        &mut self.nodes[n as usize]
            .text
            .as_mut()
            .expect("a text node")
            .link
    }

    /// Take `n` out of the chain, uncounting its bytes.
    fn unchain(&mut self, n: u32) {
        let link = *self.link_of(n);
        if !link.chained {
            return;
        }
        match link.newer {
            NO_PARENT => self.texts.newest = link.older,
            newer => self.link_of(newer).older = link.older,
        }
        match link.older {
            NO_PARENT => self.texts.oldest = link.newer,
            older => self.link_of(older).newer = link.newer,
        }
        self.texts.count -= 1;
        self.texts.bytes -= link.bytes;
        *self.link_of(n) = Link::default();
    }

    /// `n` was just measured or laid out: count its shaping's bytes again,
    /// make it the newest, and drop the oldest others' shaping while the
    /// bytes are over the budget.
    pub(crate) fn touched_text(&mut self, n: u32) {
        let t = self.nodes[n as usize].text.as_ref().expect("a text node");
        let Some(prepared) = &t.prepared else {
            return;
        };
        let bytes = prepared.heap_bytes() as u64;
        self.unchain(n);
        let newest = self.texts.newest;
        *self.link_of(n) = Link {
            newer: NO_PARENT,
            older: newest,
            chained: true,
            bytes,
        };
        match newest {
            NO_PARENT => self.texts.oldest = n,
            newest => self.link_of(newest).newer = n,
        }
        self.texts.newest = n;
        self.texts.count += 1;
        self.texts.bytes += bytes;
        self.keep_to_text_budget(n);
    }

    /// Drop the shaping of the least recently laid out texts, other than
    /// `keep`, while the bytes are over the budget.
    fn keep_to_text_budget(&mut self, keep: u32) {
        while self.texts.bytes > self.texts.budget {
            let oldest = self.texts.oldest;
            if oldest == NO_PARENT || oldest == keep {
                break;
            }
            self.forget_text(oldest);
            self.texts.dropped += 1;
        }
    }

    /// Drop `n`'s shaping (its drawn layout stays), out of the chain.
    pub(crate) fn forget_text(&mut self, n: u32) {
        self.unchain(n);
        if let Some(t) = self.nodes[n as usize].text.as_mut() {
            t.prepared = None;
        }
    }

    /// Keep about `bytes` of the texts' shaping (64 MB by default): past
    /// it, the shaping of the texts laid out longest ago is dropped, and
    /// such a text is shaped again when it is measured again. What they are
    /// drawn with is kept either way.
    pub fn set_text_budget(&mut self, bytes: u64) {
        self.texts.budget = bytes;
        self.keep_to_text_budget(NO_PARENT);
    }
}
