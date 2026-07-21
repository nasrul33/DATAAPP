#[expect(
    dead_code,
    reason = "the queue is crate-private infrastructure that later executor tasks consume; this isolated task exposes no public executor API"
)]
mod queue;

pub mod resource;
