use std::sync::atomic::{AtomicBool, Ordering};

unsafe extern "C" {
    fn nxshim_audio_init(rate: *mut u32, channels: *mut u32) -> i32;
    fn nxshim_audio_open_buffers(frames_per_buffer: usize, count: usize) -> i32;
    fn nxshim_audio_acquire() -> *mut i16;
    fn nxshim_audio_submit(data: *mut i16, bytes: usize) -> i32;
    fn nxshim_audio_close();
}

static OUTPUT_LOCK: AtomicBool = AtomicBool::new(false);

/// One libnx audout output; moved into the audio thread and closed on drop.
pub struct Output {
    rate: u32,
    channels: usize,
    frames_per_buffer: usize,
    closed: bool,
}

impl Output {
    pub fn open() -> Result<Self, String> {
        if OUTPUT_LOCK.swap(true, Ordering::SeqCst) {
            return Err("another audio output is already open".to_owned());
        }

        let mut rate = 0;
        let mut channels = 0;
        if unsafe { nxshim_audio_init(&mut rate, &mut channels) } != 0 {
            OUTPUT_LOCK.store(false, Ordering::SeqCst);
            return Err("libnx audout init failed".to_owned());
        }

        let channels = channels.max(1).min(2) as usize;
        let frames_per_buffer = 2048;
        if unsafe { nxshim_audio_open_buffers(frames_per_buffer, 4) } != 0 {
            unsafe { nxshim_audio_close() };
            OUTPUT_LOCK.store(false, Ordering::SeqCst);
            return Err("libnx audout buffer setup failed".to_owned());
        }

        Ok(Self {
            rate,
            channels,
            frames_per_buffer,
            closed: false,
        })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn frames_per_buffer(&self) -> usize {
        self.frames_per_buffer
    }

    pub fn play(&mut self, mut fill: impl FnMut(&mut [i16])) -> bool {
        if self.closed {
            return false;
        }

        let data = unsafe { nxshim_audio_acquire() };
        if data.is_null() {
            return false;
        }

        let len = self.frames_per_buffer * self.channels;
        let buf = unsafe { std::slice::from_raw_parts_mut(data, len) };
        fill(buf);

        let bytes = len * std::mem::size_of::<i16>();
        let ok = unsafe { nxshim_audio_submit(data, bytes) } == 0;
        if !ok {
            self.closed = true;
        }
        ok
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        // SAFETY: libnx's audout service owns the ring buffers and must be shut down once per
        // open session; a second close is already guarded inside nxshim.
        unsafe { nxshim_audio_close() };
        OUTPUT_LOCK.store(false, Ordering::SeqCst);
    }
}

unsafe impl Send for Output {}
