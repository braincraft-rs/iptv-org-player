//! libmpv wrapper: owns the mpv handle and renders video into a GL texture
//! that egui can display.

use std::ffi::{CStr, c_void};
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow};
use glow::HasContext;
use libmpv2::events::Event;
use libmpv2::render::{OpenGLInitParams, RenderContext, RenderParam, RenderParamApiType};
use libmpv2::{Mpv, mpv_end_file_reason};

/// What we need to know to open a stream.
#[derive(Debug, Clone, Default)]
pub struct StreamSpec {
    pub url: String,
    pub referrer: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerStatus {
    Idle,
    Loading,
    Playing,
    Error(String),
}

pub type GlLoader = Arc<dyn Fn(&CStr) -> *const c_void + Send + Sync>;

fn get_proc_address(loader: &GlLoader, name: &str) -> *mut c_void {
    match std::ffi::CString::new(name) {
        Ok(name) => loader(&name) as *mut c_void,
        Err(_) => std::ptr::null_mut(),
    }
}

struct Target {
    fbo: glow::Framebuffer,
    tex: glow::Texture,
    size: [i32; 2],
}

pub struct Player {
    // The render context borrows the Mpv handle. The handle is leaked on purpose:
    // there is exactly one player for the whole app lifetime.
    mpv: &'static Mpv,
    events: Mpv,
    render: RenderContext<'static>,
    gl: Arc<glow::Context>,
    target: Option<Target>,
    status: PlayerStatus,
}

impl Player {
    pub fn new(gl: Arc<glow::Context>, loader: GlLoader, egui_ctx: egui::Context) -> Result<Self> {
        let mpv = Mpv::with_initializer(|init| {
            init.set_property("vo", "libmpv")?;
            init.set_property("ytdl", false)?;
            // Dead streams should fail fast instead of hanging the UI.
            init.set_property("network-timeout", 10i64)?;
            if std::env::var_os("IPTV_MPV_DEBUG").is_some() {
                init.set_property("terminal", true)?;
                init.set_property("msg-level", "all=v")?;
            }
            Ok(())
        })
        .map_err(|e| anyhow!("mpv init failed: {e}"))?;
        let mpv: &'static Mpv = Box::leak(Box::new(mpv));

        let mut render = mpv
            .create_render_context(vec![
                RenderParam::ApiType(RenderParamApiType::OpenGl),
                RenderParam::InitParams(OpenGLInitParams {
                    get_proc_address,
                    ctx: loader,
                }),
            ])
            .map_err(|e| anyhow!("mpv render context failed: {e}"))?;
        let ctx = egui_ctx.clone();
        render.set_update_callback(move || ctx.request_repaint());

        let mut events = mpv
            .create_client(None)
            .map_err(|e| anyhow!("mpv client failed: {e}"))?;
        events.disable_deprecated_events().ok();
        let ctx = egui_ctx;
        events.set_wakeup_callback(move || ctx.request_repaint());

        Ok(Self {
            mpv,
            events,
            render,
            gl,
            target: None,
            status: PlayerStatus::Idle,
        })
    }

    pub fn status(&self) -> &PlayerStatus {
        &self.status
    }

    pub fn play(&mut self, spec: &StreamSpec) -> Result<()> {
        // Options are global in mpv, so always reset them to avoid leaking a
        // previous channel's Referer/User-Agent into the next one.
        self.mpv
            .set_property("referrer", spec.referrer.clone().unwrap_or_default())
            .map_err(|e| anyhow!("{e}"))?;
        self.mpv
            .set_property(
                "user-agent",
                spec.user_agent.clone().unwrap_or_else(|| "libmpv".into()),
            )
            .map_err(|e| anyhow!("{e}"))?;
        self.mpv
            .command("loadfile", &[&spec.url, "replace"])
            .map_err(|e| anyhow!("loadfile failed: {e}"))?;
        self.status = PlayerStatus::Loading;
        Ok(())
    }

    pub fn stop(&mut self) {
        let _ = self.mpv.command("stop", &[]);
        self.status = PlayerStatus::Idle;
    }

    pub fn set_volume(&self, volume: f64) {
        let _ = self.mpv.set_property("volume", volume.clamp(0.0, 100.0));
    }

    pub fn set_muted(&self, muted: bool) {
        let _ = self.mpv.set_property("mute", muted);
    }

    /// Drain pending mpv events and update `status`.
    pub fn poll_events(&mut self) {
        let before = self.status.clone();
        self.drain_events();
        if self.status != before {
            tracing::info!(status = ?self.status, "player status");
        }
    }

    fn drain_events(&mut self) {
        while let Some(ev) = self.events.wait_event(0.0) {
            match ev {
                Ok(Event::StartFile) => self.status = PlayerStatus::Loading,
                Ok(Event::PlaybackRestart) => self.status = PlayerStatus::Playing,
                Ok(Event::EndFile(reason)) => {
                    if reason == mpv_end_file_reason::Error {
                        self.status = PlayerStatus::Error("stream failed to play".into());
                    } else if reason != mpv_end_file_reason::Redirect
                        && !matches!(self.status, PlayerStatus::Loading)
                    {
                        self.status = PlayerStatus::Idle;
                    }
                }
                Ok(other) => tracing::trace!(?other, "mpv event"),
                Err(e) => tracing::warn!("mpv event error: {e}"),
            }
        }
    }

    /// Render the current frame into an offscreen texture of `size` pixels and
    /// return the GL texture so egui can draw it.
    pub fn render_frame(&mut self, size: [i32; 2]) -> Result<glow::Texture> {
        let size = [size[0].max(1), size[1].max(1)];
        let gl = self.gl.clone();
        if self.target.as_ref().is_none_or(|t| t.size != size) {
            self.destroy_target();
            self.target = Some(unsafe { create_target(&gl, size)? });
        }
        let target = self.target.as_ref().context("no render target")?;
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.fbo));
        }
        let res =
            self.render
                .render::<GlLoader>(target.fbo.0.get() as i32, size[0], size[1], false);
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        res.map_err(|e| anyhow!("mpv render failed: {e}"))?;
        Ok(target.tex)
    }

    fn destroy_target(&mut self) {
        if let Some(t) = self.target.take() {
            unsafe {
                self.gl.delete_framebuffer(t.fbo);
                self.gl.delete_texture(t.tex);
            }
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.destroy_target();
    }
}

unsafe fn create_target(gl: &glow::Context, size: [i32; 2]) -> Result<Target> {
    unsafe {
        let tex = gl.create_texture().map_err(|e| anyhow!(e))?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            size[0],
            size[1],
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        let fbo = gl.create_framebuffer().map_err(|e| anyhow!(e))?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(tex),
            0,
        );
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.bind_texture(glow::TEXTURE_2D, None);
        Ok(Target { fbo, tex, size })
    }
}
