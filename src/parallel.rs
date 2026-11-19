use anyhow::Result;
use std::thread::{self, ScopedJoinHandle};

pub fn join<T>(handle: ScopedJoinHandle<'_, Result<T>>) -> Result<T> {
    handle
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

pub fn map<T: Sync, U: Send>(items: &[T], load: impl Fn(&T) -> Result<U> + Sync) -> Result<Vec<U>> {
    let threads = thread::available_parallelism().map_or(4, usize::from);
    let load = &load;
    thread::scope(|s| {
        let parts: Vec<_> = items
            .chunks(items.len().div_ceil(threads).max(1))
            .map(|chunk| s.spawn(move || chunk.iter().map(load).collect::<Result<Vec<_>>>()))
            .collect();
        let parts = parts.into_iter().map(join).collect::<Result<Vec<_>>>()?;
        Ok(parts.into_iter().flatten().collect())
    })
}
