//! x86Code RAM 8: a census of what live pipeline states hold, for a long-lived consumer's memory
//! line (mtlvk's `MTLVK_MEMSTATS`): not part of the API.
//!
//! - `VkPipeline`s alive in the process (graphics pipeline libraries included), counted at the
//!   device's routed creation (`device/compiles.rs`) and at their release (`release.rs`).
//! - Graphics pipeline libraries made and not yet destroyed (`pipeline/libraries.rs`).
//! - Render variants alive, and the bytes their compiled pipelines keep
//!   (`RenderPipeline::footprint`, shared pipelines counted once per variant).

use std::sync::atomic::{AtomicI64, Ordering};

pub(crate) static PIPELINES: AtomicI64 = AtomicI64::new(0);
pub(crate) static LIBRARIES: AtomicI64 = AtomicI64::new(0);
pub(crate) static RENDER_VARIANTS: AtomicI64 = AtomicI64::new(0);
pub(crate) static RENDER_VARIANT_BYTES: AtomicI64 = AtomicI64::new(0);

/// Seconds since the process's first ask: the clock of the idle trims.
pub(crate) fn now_secs() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_secs()
}

pub(crate) fn add(counter: &AtomicI64, n: i64) {
    counter.fetch_add(n, Ordering::Relaxed);
}

/// One render variant's share of the census, given back as it drops.
pub(crate) struct CountedVariant(i64);

impl CountedVariant {
    pub(crate) fn new(bytes: u64) -> CountedVariant {
        add(&RENDER_VARIANTS, 1);
        add(&RENDER_VARIANT_BYTES, bytes as i64);
        CountedVariant(bytes as i64)
    }
}

impl Drop for CountedVariant {
    fn drop(&mut self) {
        add(&RENDER_VARIANTS, -1);
        add(&RENDER_VARIANT_BYTES, -self.0);
    }
}

/// The census as one line's fields.
pub(crate) fn summary() -> String {
    let get = |c: &AtomicI64| c.load(Ordering::Relaxed);
    format!(
        "vk pipelines {} (libraries {}) render variants {} ({} MB compiled)",
        get(&PIPELINES),
        get(&LIBRARIES),
        get(&RENDER_VARIANTS),
        get(&RENDER_VARIANT_BYTES) >> 20
    )
}
