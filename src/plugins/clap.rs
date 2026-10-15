//! clap hosting on top of clack-host.
//!
//! - `ClapPlugin` lives on the gui (main) thread: instance, editor, state,
//!   timers, fds, parameter list.
//! - `ClapUnit` is the audio side, moved into the engine. its buffers and
//!   event queue are allocated up front; parameter edits from the gui reach
//!   it through an rtrb ring and become clap param events.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::CString;
use std::os::fd::RawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use clack_extensions::audio_ports::{
    AudioPortInfoBuffer, AudioPortRescanFlags, HostAudioPorts, HostAudioPortsImpl, PluginAudioPorts,
};
use clack_extensions::gui::{
    GuiApiType, GuiConfiguration, GuiSize, HostGui, HostGuiImpl, PluginGui,
};
use clack_extensions::log::{HostLog, HostLogImpl, LogSeverity};
use clack_extensions::params::{
    HostParams, HostParamsImplMainThread, HostParamsImplShared, ParamClearFlags, ParamInfoBuffer,
    ParamInfoFlags, ParamRescanFlags, PluginParams,
};
use clack_extensions::posix_fd::{FdFlags, HostPosixFd, HostPosixFdImpl, PluginPosixFd};
use clack_extensions::state::{HostState, HostStateImpl, PluginState};
use clack_extensions::timer::{HostTimer, HostTimerImpl, PluginTimer, TimerId};
use clack_host::events::event_types::ParamValueEvent;
use clack_host::prelude::*;

use super::scan::PluginInfo;
use crate::audio::{MAX_BLOCK, SAMPLE_RATE};
use crate::engine::Unit;

/// most channels we feed a plugin per direction
const MAX_CHANNELS: usize = 8;
/// parameter changes queued per plugin between two audio blocks
const PARAM_QUEUE: usize = 1024;

// --- host handlers ---------------------------------------------------------

pub struct FsaHost;

impl HostHandlers for FsaHost {
    type Shared<'a> = HostShared;
    type MainThread<'a> = HostMain<'a>;
    type AudioProcessor<'a> = ();

    fn declare_extensions(builder: &mut HostExtensions<Self>, _shared: &Self::Shared<'_>) {
        builder
            .register::<HostLog>()
            .register::<HostGui>()
            .register::<HostTimer>()
            .register::<HostPosixFd>()
            .register::<HostParams>()
            .register::<HostState>()
            .register::<HostAudioPorts>();
    }
}

/// requests from the plugin, possibly from any thread. polled by the gui.
#[derive(Default)]
pub struct Requests {
    callback: AtomicBool,
    gui_closed: AtomicBool,
    state_dirty: AtomicBool,
    params_rescan: AtomicBool,
    /// width << 32 | height, 0 = none
    resize: AtomicU64,
}

pub struct HostShared {
    requests: Arc<Requests>,
    name: String,
}

impl<'a> SharedHandler<'a> for HostShared {
    fn request_restart(&self) {
        // restarting would mean reactivating; the plugin keeps running as is
        log::info!("{}: restart requested (ignored)", self.name);
    }

    fn request_process(&self) {
        // we always process
    }

    fn request_callback(&self) {
        self.requests.callback.store(true, Ordering::Release);
    }
}

impl HostLogImpl for HostShared {
    fn log(&self, severity: LogSeverity, message: &str) {
        // note: plugins may log from the audio thread; that's on them
        match severity {
            LogSeverity::Debug => log::debug!("[{}] {message}", self.name),
            LogSeverity::Info => log::info!("[{}] {message}", self.name),
            LogSeverity::Warning => log::warn!("[{}] {message}", self.name),
            _ => log::error!("[{}] {message}", self.name),
        }
    }
}

impl HostGuiImpl for HostShared {
    fn resize_hints_changed(&self) {}

    fn request_resize(&self, size: GuiSize) -> Result<(), HostError> {
        let packed = ((size.width as u64) << 32) | size.height as u64;
        self.requests.resize.store(packed.max(1), Ordering::Release);
        Ok(())
    }

    fn request_show(&self) -> Result<(), HostError> {
        Ok(())
    }

    fn request_hide(&self) -> Result<(), HostError> {
        Ok(())
    }

    fn closed(&self, _was_destroyed: bool) {
        self.requests.gui_closed.store(true, Ordering::Release);
    }
}

impl HostParamsImplShared for HostShared {
    fn request_flush(&self) {
        // the plugin is always processing, so changes get flushed anyway
    }
}

struct Timer {
    id: TimerId,
    period: Duration,
    next: Instant,
}

pub struct HostMain<'a> {
    shared: &'a HostShared,
    next_timer: Cell<u32>,
    timers: RefCell<Vec<Timer>>,
    fds: RefCell<Vec<(RawFd, FdFlags)>>,
    gui: Cell<Option<PluginGui>>,
    timer_ext: Cell<Option<PluginTimer>>,
    fd_ext: Cell<Option<PluginPosixFd>>,
    params: Cell<Option<PluginParams>>,
    state: Cell<Option<PluginState>>,
    audio_ports: Cell<Option<PluginAudioPorts>>,
}

impl<'a> HostMain<'a> {
    fn new(shared: &'a HostShared) -> Self {
        Self {
            shared,
            next_timer: Cell::new(1),
            timers: RefCell::default(),
            fds: RefCell::default(),
            gui: Cell::new(None),
            timer_ext: Cell::new(None),
            fd_ext: Cell::new(None),
            params: Cell::new(None),
            state: Cell::new(None),
            audio_ports: Cell::new(None),
        }
    }
}

impl<'a> MainThreadHandler<'a> for HostMain<'a> {
    fn initialized(&self, instance: InitializedPluginHandle<'a>) {
        self.gui.set(instance.get_extension());
        self.timer_ext.set(instance.get_extension());
        self.fd_ext.set(instance.get_extension());
        self.params.set(instance.get_extension());
        self.state.set(instance.get_extension());
        self.audio_ports.set(instance.get_extension());
    }
}

impl HostTimerImpl for HostMain<'_> {
    fn register_timer(&self, period_ms: u32) -> Result<TimerId, HostError> {
        let id = TimerId(self.next_timer.get());
        self.next_timer.set(id.0 + 1);
        // the gui ticks at ~60 hz at best; clamp silly periods
        let period = Duration::from_millis(period_ms.max(10) as u64);
        self.timers.borrow_mut().push(Timer {
            id,
            period,
            next: Instant::now() + period,
        });
        Ok(id)
    }

    fn unregister_timer(&self, timer_id: TimerId) -> Result<(), HostError> {
        self.timers.borrow_mut().retain(|t| t.id != timer_id);
        Ok(())
    }
}

impl HostPosixFdImpl for HostMain<'_> {
    fn register_fd(&self, fd: RawFd, flags: FdFlags) -> Result<(), HostError> {
        let mut fds = self.fds.borrow_mut();
        fds.retain(|(f, _)| *f != fd);
        fds.push((fd, flags));
        Ok(())
    }

    fn modify_fd(&self, fd: RawFd, flags: FdFlags) -> Result<(), HostError> {
        self.register_fd(fd, flags)
    }

    fn unregister_fd(&self, fd: RawFd) -> Result<(), HostError> {
        self.fds.borrow_mut().retain(|(f, _)| *f != fd);
        Ok(())
    }
}

impl HostParamsImplMainThread for HostMain<'_> {
    fn rescan(&self, _flags: ParamRescanFlags) {
        self.shared
            .requests
            .params_rescan
            .store(true, Ordering::Release);
    }

    fn clear(&self, _param_id: ClapId, _flags: ParamClearFlags) {}
}

impl HostStateImpl for HostMain<'_> {
    fn mark_dirty(&self) {
        self.shared
            .requests
            .state_dirty
            .store(true, Ordering::Release);
    }
}

impl HostAudioPortsImpl for HostMain<'_> {
    fn is_rescan_flag_supported(&self, _flag: AudioPortRescanFlags) -> bool {
        false
    }

    fn rescan(&self, _flags: AudioPortRescanFlags) {}
}

fn host_info() -> HostInfo {
    HostInfo::new(
        "fsa",
        "fsa",
        "https://github.com/anoscetia/fsa",
        env!("CARGO_PKG_VERSION"),
    )
    .expect("static host info")
}

// --- main-thread side ------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Param {
    pub id: u32,
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub stepped: bool,
}

enum Editor {
    #[cfg(target_os = "linux")]
    Embedded {
        window: super::x11::HostWindow,
        resizable: bool,
    },
    Floating,
}

pub struct ClapPlugin {
    pub info: PluginInfo,
    instance: PluginInstance<FsaHost>,
    requests: Arc<Requests>,
    params_tx: rtrb::Producer<(u32, f64)>,
    pub params: Vec<Param>,
    editor: Option<Editor>,
    gui_config: Option<GuiConfiguration<'static>>,
    last_state_poll: Instant,
    /// values just sent to the audio thread, shown until the plugin reports them
    recent_edits: HashMap<u32, (f64, Instant)>,
}

impl ClapPlugin {
    /// instantiates and activates a plugin. the returned unit goes to the engine.
    pub fn load(info: &PluginInfo, state: Option<&[u8]>) -> anyhow::Result<(Self, ClapUnit)> {
        // safety: loading a plugin runs its code; that is the point of a host
        let entry = unsafe { PluginEntry::load(&info.path) }
            .map_err(|e| anyhow!("loading {}: {e}", info.path.display()))?;
        let id = CString::new(info.id.as_str())?;
        let requests = Arc::new(Requests::default());
        let name = info.name.clone();
        let mut instance = PluginInstance::<FsaHost>::new(
            |_| HostShared {
                requests: requests.clone(),
                name,
            },
            |shared| HostMain::new(shared),
            &entry,
            &id,
            &host_info(),
        )
        .map_err(|e| anyhow!("instantiating {}: {e}", info.id))?;

        let (in_channels, out_channels) = main_port_channels(&mut instance)?;
        if let Some(bytes) = state {
            let ext = instance.access_handler(|h| h.state.get());
            match ext {
                Some(ext) => {
                    if let Err(e) = ext.load(&instance.plugin_handle(), &mut &bytes[..]) {
                        log::warn!("{}: restoring state failed: {e}", info.name);
                    }
                }
                None => log::warn!(
                    "{}: saved state ignored, plugin has no state extension",
                    info.name
                ),
            }
        }

        let config = PluginAudioConfiguration {
            sample_rate: SAMPLE_RATE as f64,
            min_frames_count: 1,
            max_frames_count: MAX_BLOCK as u32,
        };
        let processor = instance
            .activate(|_, _| (), config)
            .map_err(|e| anyhow!("activating {}: {e}", info.id))?;

        let (params_tx, params_rx) = rtrb::RingBuffer::new(PARAM_QUEUE);
        let unit = ClapUnit::new(processor, in_channels, out_channels, params_rx);
        let mut plugin = Self {
            info: info.clone(),
            instance,
            requests,
            params_tx,
            params: Vec::new(),
            editor: None,
            gui_config: None,
            last_state_poll: Instant::now(),
            recent_edits: HashMap::new(),
        };
        plugin.gui_config = plugin.negotiate_gui();
        plugin.rescan_params();
        log::info!(
            "loaded {} ({in_channels} in / {out_channels} out)",
            info.name
        );
        Ok((plugin, unit))
    }

    fn negotiate_gui(&mut self) -> Option<GuiConfiguration<'static>> {
        let gui = self.instance.access_handler(|h| h.gui.get())?;
        let handle = self.instance.plugin_handle();
        if cfg!(target_os = "linux") {
            // prefer embedding into our own x11 window, else let the plugin float its own
            [false, true]
                .into_iter()
                .map(|is_floating| GuiConfiguration {
                    api_type: GuiApiType::X11,
                    is_floating,
                })
                .find(|c| gui.is_api_supported(&handle, *c))
        } else {
            None
        }
    }

    pub fn has_editor(&self) -> bool {
        self.gui_config.is_some()
    }

    pub fn editor_open(&self) -> bool {
        self.editor.is_some()
    }

    pub fn open_editor(&mut self) -> anyhow::Result<()> {
        if self.editor.is_some() {
            return Ok(());
        }
        let config = self.gui_config.context("plugin has no supported editor")?;
        let gui = self
            .instance
            .access_handler(|h| h.gui.get())
            .context("no gui extension")?;
        let handle = self.instance.plugin_handle();
        gui.create(&handle, config)
            .map_err(|e| anyhow!("creating editor: {e}"))?;
        let title = CString::new(format!("{} – fsa", self.info.name)).unwrap_or_default();
        if config.is_floating {
            gui.suggest_title(&handle, &title);
            if let Err(e) = gui.show(&handle) {
                gui.destroy(&handle);
                bail!("showing editor: {e}");
            }
            self.editor = Some(Editor::Floating);
            return Ok(());
        }
        #[cfg(target_os = "linux")]
        {
            let size = gui.get_size(&handle).unwrap_or(GuiSize {
                width: 640,
                height: 480,
            });
            let resizable = gui.can_resize(&handle);
            let window = match super::x11::HostWindow::new(
                title.to_str().unwrap_or("plugin"),
                size.width,
                size.height,
            ) {
                Ok(w) => w,
                Err(e) => {
                    gui.destroy(&handle);
                    return Err(e);
                }
            };
            let parent = clack_extensions::gui::Window::from_x11_handle(window.id());
            // safety: the window outlives the plugin gui; we destroy the gui before dropping it
            if let Err(e) = unsafe { gui.set_parent(&handle, parent) } {
                gui.destroy(&handle);
                bail!("embedding editor: {e}");
            }
            // some plugins only draw after show(), others return an error here
            let _ = gui.show(&handle);
            self.editor = Some(Editor::Embedded { window, resizable });
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        bail!("embedded editors are only supported on linux")
    }

    pub fn close_editor(&mut self) {
        if self.editor.take().is_some()
            && let Some(gui) = self.instance.access_handler(|h| h.gui.get())
        {
            gui.destroy(&self.instance.plugin_handle());
        }
        // the plugin likely changed something while the editor was open
        self.requests.state_dirty.store(true, Ordering::Release);
    }

    pub fn rescan_params(&mut self) {
        let Some(ext) = self.instance.access_handler(|h| h.params.get()) else {
            return;
        };
        let handle = self.instance.plugin_handle();
        let mut buffer = ParamInfoBuffer::new();
        self.params = (0..ext.count(&handle))
            .filter_map(|i| {
                let p = ext.get_info(&handle, i, &mut buffer)?;
                if p.flags
                    .intersects(ParamInfoFlags::IS_HIDDEN | ParamInfoFlags::IS_READONLY)
                {
                    return None;
                }
                Some(Param {
                    id: p.id.get(),
                    name: String::from_utf8_lossy(p.name)
                        .trim_end_matches('\0')
                        .to_string(),
                    min: p.min_value,
                    max: p.max_value,
                    default: p.default_value,
                    stepped: p.flags.contains(ParamInfoFlags::IS_STEPPED),
                })
            })
            .collect();
    }

    pub fn param_value(&mut self, id: u32) -> Option<f64> {
        if let Some(&(v, at)) = self.recent_edits.get(&id) {
            if at.elapsed() < Duration::from_millis(500) {
                return Some(v);
            }
            self.recent_edits.remove(&id);
        }
        let ext = self.instance.access_handler(|h| h.params.get())?;
        ext.get_value(&self.instance.plugin_handle(), ClapId::new(id))
    }

    pub fn param_text(&mut self, id: u32, value: f64) -> Option<String> {
        let ext = self.instance.access_handler(|h| h.params.get())?;
        let mut buf = [0u8; 128];
        let text = ext
            .value_to_text(
                &self.instance.plugin_handle(),
                ClapId::new(id),
                value,
                &mut buf,
            )
            .ok()?;
        let end = text.iter().position(|&b| b == 0).unwrap_or(text.len());
        Some(String::from_utf8_lossy(&text[..end]).into_owned())
    }

    /// queues a parameter change for the audio thread
    pub fn set_param(&mut self, id: u32, value: f64) {
        self.recent_edits.insert(id, (value, Instant::now()));
        if self.params_tx.push((id, value)).is_err() {
            log::warn!("{}: parameter queue full", self.info.name);
        }
        self.requests.state_dirty.store(true, Ordering::Release);
    }

    pub fn save_state(&mut self) -> Option<Vec<u8>> {
        let ext = self.instance.access_handler(|h| h.state.get())?;
        let mut bytes = Vec::new();
        match ext.save(&self.instance.plugin_handle(), &mut bytes) {
            Ok(()) => Some(bytes),
            Err(e) => {
                log::warn!("{}: saving state failed: {e}", self.info.name);
                None
            }
        }
    }

    /// main-thread housekeeping, call every gui frame. returns true when the
    /// plugin state may have changed and should be re-saved.
    pub fn poll(&mut self) -> bool {
        if self.requests.callback.swap(false, Ordering::AcqRel) {
            self.instance.call_on_main_thread_callback();
        }
        if self.requests.params_rescan.swap(false, Ordering::AcqRel) {
            self.rescan_params();
        }
        self.tick_timers();
        self.poll_fds();
        self.poll_editor();

        let mut dirty = self.requests.state_dirty.swap(false, Ordering::AcqRel);
        // not every plugin calls mark_dirty after edits in its own editor
        if self.editor.is_some() && self.last_state_poll.elapsed() > Duration::from_secs(2) {
            self.last_state_poll = Instant::now();
            dirty = true;
        }
        dirty
    }

    fn tick_timers(&mut self) {
        let Some(ext) = self.instance.access_handler(|h| h.timer_ext.get()) else {
            return;
        };
        let now = Instant::now();
        let due: Vec<TimerId> = self.instance.access_handler(|h| {
            let mut timers = h.timers.borrow_mut();
            timers
                .iter_mut()
                .filter(|t| t.next <= now)
                .map(|t| {
                    t.next = now + t.period;
                    t.id
                })
                .collect()
        });
        let handle = self.instance.plugin_handle();
        for id in due {
            ext.on_timer(&handle, id);
        }
    }

    fn poll_fds(&mut self) {
        let Some(ext) = self.instance.access_handler(|h| h.fd_ext.get()) else {
            return;
        };
        let fds = self.instance.access_handler(|h| h.fds.borrow().clone());
        if fds.is_empty() {
            return;
        }
        let mut pollfds: Vec<libc::pollfd> = fds
            .iter()
            .map(|(fd, flags)| {
                let mut events = 0;
                if flags.contains(FdFlags::READ) {
                    events |= libc::POLLIN;
                }
                if flags.contains(FdFlags::WRITE) {
                    events |= libc::POLLOUT;
                }
                libc::pollfd {
                    fd: *fd,
                    events,
                    revents: 0,
                }
            })
            .collect();
        // safety: valid pollfd array, zero timeout
        let n = unsafe { libc::poll(pollfds.as_mut_ptr(), pollfds.len() as libc::nfds_t, 0) };
        if n <= 0 {
            return;
        }
        let handle = self.instance.plugin_handle();
        for p in pollfds.iter().filter(|p| p.revents != 0) {
            let mut flags = FdFlags::empty();
            if p.revents & libc::POLLIN != 0 {
                flags |= FdFlags::READ;
            }
            if p.revents & libc::POLLOUT != 0 {
                flags |= FdFlags::WRITE;
            }
            if p.revents & (libc::POLLERR | libc::POLLHUP) != 0 {
                flags |= FdFlags::ERROR;
            }
            ext.on_fd(&handle, p.fd, flags);
        }
    }

    fn poll_editor(&mut self) {
        if self.requests.gui_closed.swap(false, Ordering::AcqRel) {
            self.close_editor();
            return;
        }
        let resize = self.requests.resize.swap(0, Ordering::AcqRel);
        #[cfg(target_os = "linux")]
        if let Some(Editor::Embedded { window, resizable }) = &mut self.editor {
            let gui = self.instance.access_handler(|h| h.gui.get());
            let Some(gui) = gui else { return };
            if resize != 0 {
                window.resize((resize >> 32) as u32, resize as u32);
            }
            let mut close = false;
            for ev in window.poll() {
                match ev {
                    super::x11::WindowEvent::CloseRequested => close = true,
                    super::x11::WindowEvent::Resized(w, h) if *resizable => {
                        let handle = self.instance.plugin_handle();
                        let want = GuiSize {
                            width: w,
                            height: h,
                        };
                        let size = gui.adjust_size(&handle, want).unwrap_or(want);
                        let _ = gui.set_size(&handle, size);
                        if size != want {
                            window.resize(size.width, size.height);
                        }
                    }
                    super::x11::WindowEvent::Resized(..) => {}
                }
            }
            if close {
                self.close_editor();
            }
        }
        let _ = resize;
    }

    /// takes the processor back from a unit the engine returned and deactivates
    pub fn deactivate(mut self, unit: ClapUnit) {
        self.close_editor();
        match unit.state {
            Proc::Stopped(p) => self.instance.deactivate(p),
            Proc::Started(p) => self.instance.deactivate(p.stop_processing()),
            Proc::Empty => {}
        }
    }
}

/// channel counts of the main input and output ports
fn main_port_channels(instance: &mut PluginInstance<FsaHost>) -> anyhow::Result<(usize, usize)> {
    let ext = instance
        .access_handler(|h| h.audio_ports.get())
        .context("plugin declares no audio ports (instruments aren't supported)")?;
    let handle = instance.plugin_handle();
    let mut buffer = AudioPortInfoBuffer::default();
    let mut channels = |is_input| {
        if ext.count(&handle, is_input) == 0 {
            return 0;
        }
        ext.get(&handle, 0, is_input, &mut buffer)
            .map_or(0, |p| p.channel_count as usize)
    };
    let (i, o) = (channels(true), channels(false));
    if o == 0 {
        bail!("plugin has no audio output");
    }
    Ok((i.min(MAX_CHANNELS), o.min(MAX_CHANNELS)))
}

// --- audio side ------------------------------------------------------------

enum Proc {
    Stopped(StoppedPluginAudioProcessor<FsaHost>),
    Started(StartedPluginAudioProcessor<FsaHost>),
    Empty,
}

pub struct ClapUnit {
    state: Proc,
    in_ports: AudioPorts,
    out_ports: AudioPorts,
    in_bufs: Vec<Box<[f32]>>,
    out_bufs: Vec<Box<[f32]>>,
    events: EventBuffer,
    params_rx: rtrb::Consumer<(u32, f64)>,
    steady: u64,
}

impl ClapUnit {
    fn new(
        processor: StoppedPluginAudioProcessor<FsaHost>,
        in_channels: usize,
        out_channels: usize,
        params_rx: rtrb::Consumer<(u32, f64)>,
    ) -> Self {
        let buf = || vec![0.0f32; MAX_BLOCK].into_boxed_slice();
        Self {
            state: Proc::Stopped(processor),
            in_ports: AudioPorts::with_capacity(in_channels, 1),
            out_ports: AudioPorts::with_capacity(out_channels, 1),
            in_bufs: (0..in_channels).map(|_| buf()).collect(),
            out_bufs: (0..out_channels).map(|_| buf()).collect(),
            events: EventBuffer::with_capacity(PARAM_QUEUE),
            params_rx,
            steady: 0,
        }
    }

    fn started(&mut self) -> Option<&mut StartedPluginAudioProcessor<FsaHost>> {
        if let Proc::Stopped(_) = self.state {
            let Proc::Stopped(p) = std::mem::replace(&mut self.state, Proc::Empty) else {
                unreachable!()
            };
            match p.start_processing() {
                Ok(started) => self.state = Proc::Started(started),
                // the plugin refused; leave it stopped and pass audio through
                Err(e) => self.state = Proc::Stopped(e.into_stopped_processor()),
            }
        }
        match &mut self.state {
            Proc::Started(p) => Some(p),
            _ => None,
        }
    }
}

impl Unit for ClapUnit {
    fn process(&mut self, buf: &mut [f32]) {
        // start_processing must happen on the audio thread, so do it lazily
        if self.started().is_none() {
            return;
        }
        let n = buf.len();
        let Self {
            state,
            in_ports,
            out_ports,
            in_bufs,
            out_bufs,
            events,
            params_rx,
            steady,
        } = self;
        let Proc::Started(processor) = state else {
            return;
        };

        for ch in in_bufs.iter_mut() {
            ch[..n].copy_from_slice(buf);
        }
        events.clear();
        while let Ok((id, value)) = params_rx.pop() {
            // stays within the pre-allocated capacity (ring size == capacity)
            events.push(&ParamValueEvent::new(
                0,
                ClapId::new(id),
                Pckn::match_all(),
                value,
            ));
        }

        let input = in_ports.with_input_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_input_only(
                in_bufs
                    .iter_mut()
                    .map(|b| InputChannel::variable(&mut b[..n])),
            ),
        }]);
        let mut output = out_ports.with_output_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_output_only(
                out_bufs.iter_mut().map(|b| &mut b[..n]),
            ),
        }]);
        let result = processor.process(
            &input,
            &mut output,
            &InputEvents::from_buffer(events),
            &mut OutputEvents::void(),
            Some(*steady),
            None,
        );
        *steady += n as u64;
        // on error the dry signal passes through untouched
        if result.is_ok() {
            mixdown(out_bufs, buf);
        }
    }

    fn reset(&mut self) {
        if let Proc::Started(p) = &mut self.state {
            p.reset();
        }
    }

    fn stop(&mut self) {
        if let Proc::Started(_) = self.state {
            let Proc::Started(p) = std::mem::replace(&mut self.state, Proc::Empty) else {
                unreachable!()
            };
            self.state = Proc::Stopped(p.stop_processing());
        }
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }
}

/// averages the plugin's output channels back to mono
fn mixdown(outs: &[Box<[f32]>], buf: &mut [f32]) {
    let n = buf.len();
    match outs {
        [] => {}
        [mono] => buf.copy_from_slice(&mono[..n]),
        many => {
            let scale = 1.0 / many.len() as f32;
            for (i, s) in buf.iter_mut().enumerate() {
                *s = many.iter().map(|ch| ch[i]).sum::<f32>() * scale;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Command, Engine, Slot, SlotState};
    use crate::shared::Shared;

    /// hosts the plugin at $FSA_TEST_CLAP end to end. skipped when unset.
    #[test]
    fn hosts_plugin_from_env() {
        let Some(path) = std::env::var_os("FSA_TEST_CLAP") else {
            eprintln!("FSA_TEST_CLAP not set, skipping");
            return;
        };
        let infos = crate::plugins::scan::describe(path.as_ref()).unwrap();
        let info = &infos[0];
        let (mut plugin, unit) = ClapPlugin::load(info, None).unwrap();
        assert!(
            !plugin.params.is_empty(),
            "test plugin should expose parameters"
        );

        let shared = Arc::new(Shared::default());
        let state = SlotState::new(true);
        let (mut engine, mut handle) = Engine::new(shared, Vec::new(), SAMPLE_RATE as f32);
        handle.send(Command::Insert(Slot::new(
            7,
            Box::new(unit),
            state,
            SAMPLE_RATE as f32,
        )));

        let input = [0.5f32; 256];
        let mut out = [0.0f32; 256];
        engine.process(&input, &mut out);
        let before = out[255];
        assert!(before.is_finite());

        // drive the first parameter to its minimum through the audio thread
        let p = plugin.params[0].clone();
        plugin.set_param(p.id, p.min);
        for _ in 0..4 {
            engine.process(&input, &mut out);
        }
        assert_ne!(
            out[255], before,
            "parameter change should affect the output"
        );
        let saved = plugin.save_state().expect("state");

        // restore into a fresh instance
        let (mut restored, _unit) = ClapPlugin::load(info, Some(&saved)).unwrap();
        assert_eq!(restored.param_value(p.id), Some(p.min));

        handle.send(Command::Remove(7));
        for _ in 0..4 {
            engine.process(&input, &mut out);
        }
        assert_eq!(out[255], 0.5, "removed plugin no longer processes");
        let slot = handle.collect_garbage().pop().expect("slot handed back");
        let unit = slot.unit.into_any().downcast::<ClapUnit>().unwrap();
        plugin.deactivate(*unit);
    }
}
