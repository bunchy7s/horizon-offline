//! One vertex buffer for every terrain section: sections own ranges of it, so
//! a frame binds it once and draws every section with indirect multi-draw.
use std::collections::BTreeMap;

/// Ranges are rounded up to this many vertices to limit fragmentation, and
/// start at multiples of it (`flat` names sections by these blocks).
pub const GRAIN: u64 = 256;
/// The largest page, in bytes.
pub const PAGE_BYTES: u64 = 256 << 20;

/// Free vertex ranges: start -> length, never adjacent.
#[derive(Debug, PartialEq)]
struct FreeList(BTreeMap<u64, u64>);

impl FreeList {
    fn new(capacity: u64) -> Self {
        Self(BTreeMap::from([(0, capacity)]))
    }
    /// First fit.
    fn take(&mut self, len: u64) -> Option<u64> {
        let (&start, &free) = self.0.iter().find(|(_, l)| **l >= len)?;
        self.0.remove(&start);
        if free > len {
            self.0.insert(start + len, free - len);
        }
        Some(start)
    }
    fn give(&mut self, start: u64, len: u64) {
        let (mut start, mut len) = (start, len);
        if let Some((&prev, &prev_len)) = self.0.range(..start).next_back()
            && prev + prev_len == start
        {
            self.0.remove(&prev);
            start = prev;
            len += prev_len;
        }
        if let Some(next_len) = self.0.remove(&(start + len)) {
            len += next_len;
        }
        self.0.insert(start, len);
    }
    /// `give` for many ranges: one merge of the sorted ranges into the list,
    /// where giving them one by one costs a few tree lookups each.
    fn give_all(&mut self, mut ranges: Vec<(u64, u64)>) {
        if ranges.len() < 64 {
            for (start, len) in ranges {
                self.give(start, len);
            }
            return;
        }
        ranges.sort_unstable();
        let mut free = std::mem::take(&mut self.0).into_iter().peekable();
        let mut given = ranges.into_iter().peekable();
        let mut merged: Vec<(u64, u64)> = Vec::new();
        loop {
            let next = match (free.peek(), given.peek()) {
                (Some(a), Some(b)) if a.0 <= b.0 => free.next(),
                (_, Some(_)) => given.next(),
                (Some(_), None) => free.next(),
                (None, None) => break,
            };
            let (start, len) = next.expect("peeked");
            match merged.last_mut() {
                Some(last) if last.0 + last.1 == start => last.1 += len,
                _ => merged.push((start, len)),
            }
        }
        // Sorted: the tree is built in one pass.
        self.0 = merged.into_iter().collect();
    }
}

/// One vertex buffer of the arena; only the newest page grows.
struct Page {
    buffer: wgpu::Buffer,
    /// Capacity in vertices.
    capacity: u64,
    free: FreeList,
}

/// Terrain vertices in pages of at most `page_limit` vertices (devices cap
/// buffer sizes): a section's vertices sit in one page.
pub struct Arena {
    pages: Vec<Option<Page>>,
    stride: u64,
    page_limit: u64,
    initial_capacity: u64,
    usage: wgpu::BufferUsages,
    /// A page had to go to system memory (logged once).
    spilled: bool,
    /// Tests: GPU-local pages fail as if video memory were full.
    #[cfg(test)]
    video_memory_full: bool,
}

impl Arena {
    /// Vertices; `storage` also lets shaders read them as a storage buffer (a shader pack's
    /// terrain program reads the other corners of a vertex's quad for its normal).
    pub fn new(device: &wgpu::Device, stride: u64, capacity: u64, storage: bool) -> Self {
        let usage = if storage {
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE
        } else {
            wgpu::BufferUsages::VERTEX
        };
        Self::with_usage(device, stride, capacity, usage)
    }
    /// Sorted translucent indices; `storage` when compute passes read them.
    pub fn indices(device: &wgpu::Device, capacity: u64, storage: bool) -> Self {
        let usage = if storage {
            wgpu::BufferUsages::INDEX | wgpu::BufferUsages::STORAGE
        } else {
            wgpu::BufferUsages::INDEX
        };
        Self::with_usage(device, 4, capacity, usage)
    }
    fn with_usage(
        device: &wgpu::Device,
        stride: u64,
        capacity: u64,
        usage: wgpu::BufferUsages,
    ) -> Self {
        // 256 MiB pages, or less where the device's buffers are smaller.
        let bytes = device.limits().max_buffer_size.min(PAGE_BYTES);
        let page_limit = (bytes / stride) / GRAIN * GRAIN;
        let capacity = capacity.next_multiple_of(GRAIN).min(page_limit);
        Self {
            pages: Vec::new(),
            stride,
            page_limit,
            initial_capacity: capacity,
            usage,
            spilled: false,
            #[cfg(test)]
            video_memory_full: false,
        }
    }
    /// A buffer of `capacity` vertices in video memory or, when that is full
    /// and the device allows it, in system memory: Vulkan drivers fail an
    /// allocation rather than page like OpenGL ones, and wgpu's default error
    /// handler panics. `None` when neither has room.
    fn create(&mut self, device: &wgpu::Device, capacity: u64) -> Option<wgpu::Buffer> {
        let usage = self.usage | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        #[cfg(test)]
        let local = (!self.video_memory_full)
            .then(|| try_create(device, capacity * self.stride, usage))
            .flatten();
        #[cfg(not(test))]
        let local = try_create(device, capacity * self.stride, usage);
        local.or_else(|| {
            // Mappable buffers are placed in host-visible memory, which the
            // allocator takes from system memory once video memory is full.
            if !device
                .features()
                .contains(wgpu::Features::MAPPABLE_PRIMARY_BUFFERS)
            {
                return None;
            }
            let buffer = try_create(
                device,
                capacity * self.stride,
                usage | wgpu::BufferUsages::MAP_WRITE,
            )?;
            if !std::mem::replace(&mut self.spilled, true) {
                log::warn!(
                    "Horizon: video memory is full; more terrain is kept in system memory \
                     (lower the render distance if frames slow down)"
                );
            }
            Some(buffer)
        })
    }
    /// The rounded length a request of `vertices` occupies.
    fn span(vertices: u64) -> u64 {
        vertices.max(1).next_multiple_of(GRAIN)
    }
    /// Bytes of every page's buffer.
    pub fn bytes(&self) -> u64 {
        self.pages.iter().flatten().map(|p| p.capacity).sum::<u64>() * self.stride
    }
    /// The most vertices a page holds.
    pub fn page_vertices(&self) -> u64 {
        self.page_limit
    }
    pub fn has_page(&self, page: u32) -> bool {
        self.pages.get(page as usize).is_some_and(Option::is_some)
    }
    /// Each live page's buffer, by page.
    pub fn page_buffers(&self) -> impl Iterator<Item = (u32, &wgpu::Buffer)> {
        let pages = self.pages.iter().enumerate();
        pages.filter_map(|(i, p)| Some((i as u32, &p.as_ref()?.buffer)))
    }
    pub fn buffer(&self, page: u32) -> &wgpu::Buffer {
        &self.pages[page as usize]
            .as_ref()
            .expect("live arena page")
            .buffer
    }
    /// A range of `vertices`: (page, first vertex). Grows the newest page
    /// (copying its contents on the GPU, in `encoder`) or starts a new one
    /// when nothing free is large enough; `None` when no memory is left for
    /// it. Writes to the arena belong in `encoder` too, so that those
    /// recorded before a grow reach the grown page.
    pub fn alloc(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        vertices: u64,
    ) -> Option<(u32, u64)> {
        let len = Self::span(vertices).min(self.page_limit);
        for (i, page) in self.pages.iter_mut().enumerate() {
            if let Some(page) = page
                && let Some(start) = page.free.take(len)
            {
                return Some((i as u32, start));
            }
        }
        let last = self.pages.len().saturating_sub(1);
        if let Some(page) = self.pages.last().and_then(Option::as_ref)
            && page.capacity < self.page_limit
        {
            let current = page.capacity;
            let grown = (current * 2).max(current + len).min(self.page_limit);
            if (grown - current >= len || grown == self.page_limit)
                && self.grow(device, encoder, last, grown)
                && let Some(start) = self.pages[last].as_mut().unwrap().free.take(len)
            {
                return Some((last as u32, start));
            }
        }
        let capacity = if self.pages.is_empty() {
            self.initial_capacity
        } else {
            1 << 20
        }
        .max(len)
        .min(self.page_limit);
        // A page just large enough when memory is short.
        let (buffer, capacity) = match self.create(device, capacity) {
            Some(buffer) => (buffer, capacity),
            None if capacity > len => (self.create(device, len)?, len),
            None => return None,
        };
        let mut free = FreeList::new(capacity);
        let start = free.take(len).expect("a new page fits one section");
        let page = Page {
            buffer,
            capacity,
            free,
        };
        let index = self
            .pages
            .iter()
            .position(Option::is_none)
            .unwrap_or_else(|| {
                self.pages.push(None);
                self.pages.len() - 1
            });
        self.pages[index] = Some(page);
        Some((index as u32, start))
    }
    pub fn release(&mut self, page: u32, start: u64, vertices: u64) {
        let len = Self::span(vertices).min(self.page_limit);
        self.pages[page as usize]
            .as_mut()
            .expect("live arena page")
            .free
            .give(start, len);
    }
    /// `release` for many ranges (page, start, vertices) at once.
    pub fn release_all(&mut self, ranges: impl IntoIterator<Item = (u32, u64, u64)>) {
        let mut by_page: Vec<Vec<(u64, u64)>> = Vec::new();
        for (page, start, vertices) in ranges {
            let page = page as usize;
            if by_page.len() <= page {
                by_page.resize_with(page + 1, Vec::new);
            }
            by_page[page].push((start, Self::span(vertices).min(self.page_limit)));
        }
        for (page, ranges) in by_page.into_iter().enumerate() {
            if !ranges.is_empty() {
                self.pages[page]
                    .as_mut()
                    .expect("live arena page")
                    .free
                    .give_all(ranges);
            }
        }
    }
    /// Moves a page into a larger buffer; false (the page unchanged) when
    /// there is no memory for it.
    fn grow(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        page: usize,
        capacity: u64,
    ) -> bool {
        let capacity = capacity.next_multiple_of(GRAIN).min(self.page_limit);
        let Some(buffer) = self.create(device, capacity) else {
            return false;
        };
        let old = self.pages[page].as_mut().expect("live arena page");
        // Copies recorded for the old buffer run before this one.
        encoder.copy_buffer_to_buffer(&old.buffer, 0, &buffer, 0, old.capacity * self.stride);
        let previous = old.capacity;
        old.buffer = buffer;
        old.capacity = capacity;
        old.free.give(previous, capacity - previous);
        true
    }
    /// Reclaim after an upload batch, so replacing a section can reuse its range.
    pub fn trim(&mut self) {
        for page in &mut self.pages {
            if let Some(p) = page
                && p.free.0.len() == 1
                && p.free.0.get(&0) == Some(&p.capacity)
            {
                *page = None;
            }
        }
        // Keep live page indices stable; later allocations reuse interior holes.
        while self.pages.last().is_some_and(Option::is_none) {
            self.pages.pop();
        }
    }
    pub fn clear(&mut self) {
        self.pages.clear();
    }
}

/// `create_buffer` with an out-of-memory failure returned instead of raised
/// (raised errors are fatal).
fn try_create(device: &wgpu::Device, size: u64, usage: wgpu::BufferUsages) -> Option<wgpu::Buffer> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Section buffer arena"),
        size,
        usage,
        mapped_at_creation: false,
    });
    // Native error scopes resolve at once.
    pollster::block_on(scope.pop()).is_none().then_some(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        device_with(wgpu::Features::empty())
    }

    /// A device with those of `features` the adapter has.
    fn device_with(features: wgpu::Features) -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        eprintln!("arena GPU regression: {}", adapter.get_info().name);
        let required_features = adapter.features() & features;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features,
            ..Default::default()
        }))
        .ok()
    }

    /// `Arena::alloc`, with the copies of a grow submitted at once.
    fn alloc(
        arena: &mut Arena,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vertices: u64,
    ) -> Option<(u32, u64)> {
        let mut encoder = device.create_command_encoder(&Default::default());
        let range = arena.alloc(device, &mut encoder, vertices);
        queue.submit([encoder.finish()]);
        range
    }

    #[test]
    fn writes_recorded_before_a_grow_reach_the_grown_page() {
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let mut arena = Arena::indices(&device, GRAIN, false);
        let mut uploads = super::super::upload::Uploads::new(&device);
        let mut encoder = device.create_command_encoder(&Default::default());
        let (page, start) = arena.alloc(&device, &mut encoder, GRAIN).unwrap();
        uploads.buffer(
            &mut encoder,
            arena.buffer(page),
            start * 4,
            &7u32.to_le_bytes(),
        );
        // The page is full: the next range grows it, in the same encoder.
        let (grown, next) = arena.alloc(&device, &mut encoder, GRAIN).unwrap();
        assert_eq!(grown, page);
        uploads.buffer(
            &mut encoder,
            arena.buffer(grown),
            next * 4,
            &9u32.to_le_bytes(),
        );
        uploads.finish(&encoder);
        queue.submit([encoder.finish()]);
        assert_eq!(read(&device, &queue, arena.buffer(page), start * 4), 7);
        assert_eq!(read(&device, &queue, arena.buffer(page), next * 4), 9);
    }

    #[test]
    fn empty_pages_are_reclaimed_without_moving_live_ranges() {
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let mut arena = Arena::new(&device, 4, GRAIN, false);
        arena.page_limit = 2 * GRAIN;
        assert_eq!(arena.bytes(), 0);
        let (first, a) = alloc(&mut arena, &device, &queue, GRAIN).unwrap();
        let (_, b) = alloc(&mut arena, &device, &queue, GRAIN).unwrap();
        let (live, c) = alloc(&mut arena, &device, &queue, 2 * GRAIN).unwrap();
        let buffer = arena.buffer(live).clone();
        let data = [1u8, 2, 3, 4];
        queue.write_buffer(&buffer, c * 4, &data);
        assert_eq!(arena.bytes(), 4 * GRAIN * 4);
        arena.release(first, a, GRAIN);
        arena.trim();
        assert_eq!(arena.bytes(), 4 * GRAIN * 4);
        arena.release(first, b, GRAIN);
        arena.trim();
        assert_eq!(arena.bytes(), 2 * GRAIN * 4);
        assert_eq!(arena.buffer(live), &buffer);
        let (reused, d) = alloc(&mut arena, &device, &queue, GRAIN).unwrap();
        assert_eq!(reused, first);

        let (tx, rx) = std::sync::mpsc::channel();
        wgpu::util::DownloadBuffer::read_buffer(
            &device,
            &queue,
            &arena.buffer(live).slice(c * 4..c * 4 + 4),
            move |result| tx.send(result.unwrap().to_vec()).unwrap(),
        );
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert_eq!(rx.recv().unwrap(), data);
        arena.release(reused, d, GRAIN);
        arena.release(live, c, 2 * GRAIN);
        arena.trim();
        assert_eq!(arena.bytes(), 0);
        assert!(arena.pages.is_empty());
    }

    #[test]
    fn clearing_an_arena_releases_buffers_and_can_allocate_again() {
        let Some((device, queue)) = device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        for mut arena in [
            Arena::new(&device, 28, GRAIN, false),
            Arena::indices(&device, GRAIN, false),
        ] {
            alloc(&mut arena, &device, &queue, GRAIN).unwrap();
            assert_eq!(arena.bytes(), arena.stride * GRAIN);
            arena.clear();
            assert_eq!(arena.bytes(), 0);
            assert_eq!(alloc(&mut arena, &device, &queue, GRAIN), Some((0, 0)));
            assert_eq!(arena.bytes(), arena.stride * GRAIN);
        }
    }

    #[test]
    fn free_ranges_coalesce() {
        let mut free = FreeList::new(4 * GRAIN);
        let a = free.take(GRAIN).unwrap();
        let b = free.take(2 * GRAIN).unwrap();
        let c = free.take(GRAIN).unwrap();
        assert_eq!((a, b, c), (0, GRAIN, 3 * GRAIN));
        assert_eq!(free.take(1), None);
        free.give(a, GRAIN);
        free.give(c, GRAIN);
        // Two holes of one grain: a two-grain request does not fit.
        assert_eq!(free.take(2 * GRAIN), None);
        free.give(b, 2 * GRAIN);
        assert_eq!(free, FreeList::new(4 * GRAIN));
    }

    #[test]
    fn many_ranges_given_at_once_coalesce_as_one_by_one() {
        let fill = || {
            let mut free = FreeList::new(1 << 30);
            let taken: Vec<(u64, u64)> = (0..500u64)
                .map(|i| {
                    let len = GRAIN * (1 + i * 7 % 5);
                    (free.take(len).unwrap(), len)
                })
                .collect();
            for &(start, len) in taken.iter().step_by(3) {
                free.give(start, len);
            }
            (free, taken)
        };
        // Every other range of those still taken, in a scattered order.
        let (mut one_by_one, taken) = fill();
        let (mut at_once, _) = fill();
        let mut given: Vec<(u64, u64)> = (0..taken.len())
            .filter(|i| i % 3 != 0 && i % 2 == 0)
            .map(|i| taken[i])
            .collect();
        given.sort_by_key(|&(start, _)| start.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        assert!(given.len() >= 64);
        for &(start, len) in &given {
            one_by_one.give(start, len);
        }
        at_once.give_all(given);
        assert_eq!(at_once, one_by_one);
        // The rest too: everything is free again, in one range.
        let rest: Vec<(u64, u64)> = (0..taken.len())
            .filter(|i| i % 3 != 0 && i % 2 != 0)
            .map(|i| taken[i])
            .collect();
        at_once.give_all(rest);
        assert_eq!(at_once, FreeList::new(1 << 30));
    }

    #[test]
    fn first_fit_reuses_the_lowest_hole() {
        let mut free = FreeList::new(8 * GRAIN);
        let a = free.take(GRAIN).unwrap();
        let _b = free.take(GRAIN).unwrap();
        free.give(a, GRAIN);
        assert_eq!(free.take(GRAIN), Some(a));
    }

    fn read(device: &wgpu::Device, queue: &wgpu::Queue, buffer: &wgpu::Buffer, offset: u64) -> u32 {
        let (tx, rx) = std::sync::mpsc::channel();
        wgpu::util::DownloadBuffer::read_buffer(
            device,
            queue,
            &buffer.slice(offset..offset + 4),
            move |result| tx.send(result.unwrap().to_vec()).unwrap(),
        );
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        u32::from_le_bytes(rx.recv().unwrap()[..4].try_into().unwrap())
    }

    #[test]
    fn full_video_memory_spills_to_system_memory() {
        let Some((device, queue)) = device_with(wgpu::Features::MAPPABLE_PRIMARY_BUFFERS) else {
            return;
        };
        if !device
            .features()
            .contains(wgpu::Features::MAPPABLE_PRIMARY_BUFFERS)
        {
            return;
        }
        let mut arena = Arena::indices(&device, GRAIN, false);
        let (page, start) = alloc(&mut arena, &device, &queue, GRAIN).unwrap();
        queue.write_buffer(arena.buffer(page), start * 4, &7u32.to_le_bytes());
        arena.video_memory_full = true;
        // The full page can't grow in video memory: it moves to system
        // memory with its contents, and new ranges come from there.
        let (grown, next) = alloc(&mut arena, &device, &queue, GRAIN).unwrap();
        assert!(arena.spilled);
        assert_eq!(grown, page);
        assert!(
            arena
                .buffer(page)
                .usage()
                .contains(wgpu::BufferUsages::MAP_WRITE)
        );
        queue.write_buffer(arena.buffer(grown), next * 4, &9u32.to_le_bytes());
        assert_eq!(read(&device, &queue, arena.buffer(page), start * 4), 7);
        assert_eq!(read(&device, &queue, arena.buffer(grown), next * 4), 9);
    }

    #[test]
    fn no_memory_left_is_not_an_error() {
        let Some((device, queue)) = device() else {
            return;
        };
        let mut arena = Arena::indices(&device, GRAIN, false);
        alloc(&mut arena, &device, &queue, GRAIN).unwrap();
        arena.video_memory_full = true;
        // Without mappable primary buffers nothing can spill: the request
        // fails, and the arena keeps what it has.
        assert_eq!(alloc(&mut arena, &device, &queue, GRAIN), None);
        assert_eq!(arena.bytes(), GRAIN * 4);
        arena.release(0, 0, GRAIN);
        assert_eq!(alloc(&mut arena, &device, &queue, GRAIN), Some((0, 0)));
    }
}
