//! Recording: ffmpeg presets, argument building, process supervision.
//!
//! Recording runs ffmpeg as a child process (the installed binary; see
//! [`find_ffmpeg`]). "Original" copies the stream untouched; the other presets
//! re-encode to a smaller file at the cost of video/audio quality.

pub mod schedule;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use chrono::{DateTime, Local};

use crate::model::Stream;
use crate::player::queue::StreamQueue;

/// Timing rules for supervising ffmpeg. The defaults suit real streams; tests
/// shrink them.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// A stream that dies sooner than this (or writes nothing) is a failed
    /// start: the next stream of the channel is tried.
    pub fast_fail: Duration,
    /// No data at all this long after (re)starting: the stream is dead, try the next.
    pub start_stall: Duration,
    /// Output stopped growing this long mid-recording: give up, keep what we have.
    pub mid_stall: Duration,
    /// ffmpeg's `-t` counts media time, so a stalled stream would never end;
    /// stop a timed recording this long after its wall-clock end.
    pub duration_grace: Duration,
    /// How long ffmpeg gets to finalise the file after a graceful stop.
    pub stop_grace: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            fast_fail: Duration::from_secs(15),
            start_stall: Duration::from_secs(20),
            mid_stall: Duration::from_secs(90),
            duration_grace: Duration::from_secs(30),
            stop_grace: Duration::from_secs(6),
        }
    }
}

// ---- presets -----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// Copy the stream as-is: best quality, biggest file.
    Original,
    Medium,
    Small,
    Tiny,
    AudioOnly,
}

impl Preset {
    pub const ALL: [Preset; 5] = [
        Preset::Original,
        Preset::Medium,
        Preset::Small,
        Preset::Tiny,
        Preset::AudioOnly,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Preset::Original => "original",
            Preset::Medium => "medium",
            Preset::Small => "small",
            Preset::Tiny => "tiny",
            Preset::AudioOnly => "audio",
        }
    }

    pub fn from_key(key: &str) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| p.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Preset::Original => "Original (no re-encode)",
            Preset::Medium => "Medium 720p",
            Preset::Small => "Small 480p",
            Preset::Tiny => "Tiny 360p",
            Preset::AudioOnly => "Audio only",
        }
    }

    /// Rough size per hour, for the UI. Real sizes depend on the channel.
    pub fn size_hint(self) -> &'static str {
        match self {
            Preset::Original => "up to ~2 GB/h for HD",
            Preset::Medium => "~350 MB/h",
            Preset::Small => "~200 MB/h",
            Preset::Tiny => "~90 MB/h",
            Preset::AudioOnly => "~30 MB/h",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            // MPEG-TS accepts whatever codecs the stream carries.
            Preset::Original => "ts",
            Preset::AudioOnly => "m4a",
            _ => "mp4",
        }
    }
}

/// ffmpeg arguments to record `stream` into `out`, stopping after `duration`
/// if given.
pub fn build_args(
    stream: &Stream,
    preset: Preset,
    out: &Path,
    duration: Option<Duration>,
) -> Vec<String> {
    let s = |v: &str| v.to_string();
    let mut a: Vec<String> = vec![s("-hide_banner"), s("-loglevel"), s("error"), s("-y")];
    // Input options (must come before -i).
    if let Some(ua) = &stream.user_agent {
        a.extend([s("-user_agent"), ua.clone()]);
    }
    if let Some(r) = &stream.referrer {
        a.extend([s("-referer"), r.clone()]);
    }
    // Reconnect options belong to ffmpeg's http protocol; other inputs reject them.
    if stream.url.starts_with("http://") || stream.url.starts_with("https://") {
        a.extend([
            s("-reconnect"),
            s("1"),
            s("-reconnect_streamed"),
            s("1"),
            s("-reconnect_delay_max"),
            s("5"),
        ]);
    }
    a.extend([s("-i"), stream.url.clone(), s("-sn")]);
    if let Some(d) = duration {
        a.extend([s("-t"), d.as_secs().max(1).to_string()]);
    }
    let scale = |h: u32| format!("scale=-2:min({h}\\,ih),format=yuv420p");
    let h264 = |h: u32, crf: u32, audio_k: u32, channels: u32| {
        vec![
            s("-vf"),
            scale(h),
            s("-c:v"),
            s("libx264"),
            s("-preset"),
            s("veryfast"),
            s("-crf"),
            crf.to_string(),
            s("-c:a"),
            s("aac"),
            s("-b:a"),
            format!("{audio_k}k"),
            s("-ac"),
            channels.to_string(),
            // Fragmented MP4 stays playable even if the app or ffmpeg is killed.
            s("-movflags"),
            s("+frag_keyframe+empty_moov+default_base_moof"),
        ]
    };
    match preset {
        Preset::Original => a.extend([s("-c"), s("copy"), s("-f"), s("mpegts")]),
        Preset::Medium => a.extend(h264(720, 26, 96, 2)),
        Preset::Small => a.extend(h264(480, 28, 80, 2)),
        Preset::Tiny => a.extend(h264(360, 31, 48, 1)),
        Preset::AudioOnly => a.extend([
            s("-vn"),
            s("-c:a"),
            s("aac"),
            s("-b:a"),
            s("64k"),
            s("-ac"),
            s("2"),
            s("-f"),
            s("ipod"),
            s("-movflags"),
            s("+frag_keyframe+empty_moov"),
        ]),
    }
    a.push(out.to_string_lossy().into_owned());
    a
}

// ---- files -------------------------------------------------------------------

/// Channel name -> safe file name part.
pub fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c
            } else if c == ' ' || c == '-' || c == '_' {
                '_'
            } else {
                '\0'
            }
        })
        .filter(|&c| c != '\0')
        .take(60)
        .collect();
    if cleaned.is_empty() {
        "channel".to_string()
    } else {
        cleaned
    }
}

/// `<dir>/<Channel>_<YYYYMMDD_HHMMSS>.<ext>`, made unique if it already exists.
pub fn output_path(dir: &Path, channel: &str, preset: Preset, now: DateTime<Local>) -> PathBuf {
    let base = format!("{}_{}", sanitize(channel), now.format("%Y%m%d_%H%M%S"));
    let mut path = dir.join(format!("{base}.{}", preset.extension()));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{base}_{n}.{}", preset.extension()));
        n += 1;
    }
    path
}

pub fn default_output_dir() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|d| d.video_dir().map(Path::to_path_buf))
        .or_else(|| directories::UserDirs::new().map(|d| d.home_dir().to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("IPTV-Org Player")
}

/// Locate ffmpeg: the user-configured path first, then PATH, then common
/// install locations (GUI apps on macOS do not inherit the shell's PATH).
pub fn find_ffmpeg(custom: &str) -> Option<PathBuf> {
    let custom = custom.trim();
    if !custom.is_empty() {
        let p = PathBuf::from(custom);
        return p.is_file().then_some(p);
    }
    let exe = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let from_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    let extra = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from);
    from_path
        .into_iter()
        .chain(extra)
        .map(|d| d.join(exe))
        .find(|p| p.is_file())
}

pub fn install_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "ffmpeg not found. Install it with: brew install ffmpeg"
    } else if cfg!(windows) {
        "ffmpeg not found. Install it (e.g. winget install ffmpeg) or set its path below."
    } else {
        "ffmpeg not found. Install it with your package manager (e.g. sudo apt install ffmpeg)."
    }
}

/// Open the file manager with `path` selected (or its folder).
pub fn reveal(path: &Path) {
    let result = if cfg!(target_os = "macos") {
        Command::new("open").arg("-R").arg(path).spawn()
    } else if cfg!(windows) {
        Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn()
    } else {
        Command::new("xdg-open")
            .arg(path.parent().unwrap_or(Path::new(".")))
            .spawn()
    };
    if let Err(e) = result {
        tracing::warn!("could not open file manager: {e}");
    }
}

// ---- recorder ------------------------------------------------------------------

pub struct RecordRequest {
    pub channel_name: String,
    /// Streams to try, best first.
    pub streams: Vec<Stream>,
    pub start_index: usize,
    pub preset: Preset,
    pub out_dir: PathBuf,
    pub duration: Option<Duration>,
    pub schedule_id: Option<i64>,
}

pub struct Active {
    pub id: u64,
    pub channel_name: String,
    pub preset: Preset,
    pub path: PathBuf,
    pub started: Instant,
    pub started_at: DateTime<Local>,
    pub duration: Option<Duration>,
    pub schedule_id: Option<i64>,
    queue: StreamQueue,
    child: Child,
    stdin: Option<ChildStdin>,
    log: PathBuf,
    stopping: Option<Instant>,
    attempt_started: Instant,
    last_size: u64,
    last_growth: Instant,
}

impl Active {
    pub fn size_bytes(&self) -> u64 {
        std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_stopping(&self) -> bool {
        self.stopping.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct Finished {
    pub channel_name: String,
    pub preset: Preset,
    pub path: PathBuf,
    pub started_at: DateTime<Local>,
    pub size_bytes: u64,
    pub ok: bool,
    pub note: String,
    pub schedule_id: Option<i64>,
}

#[derive(Default)]
pub struct Recorder {
    active: Vec<Active>,
    next_id: u64,
    limits: Limits,
}

fn spawn_ffmpeg(
    ffmpeg: &Path,
    stream: &Stream,
    preset: Preset,
    out: &Path,
    duration: Option<Duration>,
    log: &Path,
) -> Result<(Child, ChildStdin)> {
    let log_file = std::fs::File::create(log).context("creating ffmpeg log")?;
    let mut cmd = Command::new(ffmpeg);
    cmd.args(build_args(stream, preset, out, duration))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log_file));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd
        .spawn()
        .with_context(|| format!("starting {}", ffmpeg.display()))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("no ffmpeg stdin"))?;
    Ok((child, stdin))
}

fn last_log_line(log: &Path) -> String {
    std::fs::read_to_string(log)
        .ok()
        .and_then(|t| {
            t.lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .map(String::from)
        })
        .unwrap_or_default()
}

impl Drop for Recorder {
    /// Never leave ffmpeg processes behind.
    fn drop(&mut self) {
        for a in &mut self.active {
            let _ = a.child.kill();
            let _ = a.child.wait();
        }
    }
}

impl Recorder {
    #[cfg(test)]
    pub fn with_limits(limits: Limits) -> Self {
        Self {
            active: Vec::new(),
            next_id: 0,
            limits,
        }
    }

    pub fn active(&self) -> &[Active] {
        &self.active
    }

    pub fn is_recording(&self) -> bool {
        !self.active.is_empty()
    }

    pub fn start(&mut self, ffmpeg: &Path, req: RecordRequest) -> Result<u64> {
        if req.streams.is_empty() {
            return Err(anyhow!("channel has no streams"));
        }
        std::fs::create_dir_all(&req.out_dir)
            .with_context(|| format!("creating {}", req.out_dir.display()))?;
        let now = Local::now();
        let path = output_path(&req.out_dir, &req.channel_name, req.preset, now);
        let mut queue = StreamQueue::new(req.streams);
        queue.select(req.start_index);
        let id = self.next_id;
        self.next_id += 1;
        let log = std::env::temp_dir().join(format!("iptv-org-player-ffmpeg-{id}.log"));
        let stream = queue.current().ok_or_else(|| anyhow!("no stream"))?.clone();
        let (child, stdin) = spawn_ffmpeg(ffmpeg, &stream, req.preset, &path, req.duration, &log)?;
        self.active.push(Active {
            id,
            channel_name: req.channel_name,
            preset: req.preset,
            path,
            started: Instant::now(),
            started_at: now,
            duration: req.duration,
            schedule_id: req.schedule_id,
            queue,
            child,
            stdin: Some(stdin),
            log,
            stopping: None,
            attempt_started: Instant::now(),
            last_size: 0,
            last_growth: Instant::now(),
        });
        Ok(id)
    }

    /// Ask ffmpeg to finish the file and exit ("q" on stdin).
    pub fn stop(&mut self, id: u64) {
        if let Some(a) = self.active.iter_mut().find(|a| a.id == id)
            && a.stopping.is_none()
        {
            a.stopping = Some(Instant::now());
            if let Some(stdin) = a.stdin.as_mut() {
                let _ = stdin.write_all(b"q\n");
                let _ = stdin.flush();
            }
        }
    }

    pub fn stop_all(&mut self) {
        let ids: Vec<u64> = self.active.iter().map(|a| a.id).collect();
        for id in ids {
            self.stop(id);
        }
    }

    /// Stop everything and wait (bounded) for the files to be finalised.
    pub fn shutdown(&mut self, ffmpeg: Option<&Path>) -> Vec<Finished> {
        self.stop_all();
        let mut finished = Vec::new();
        let deadline = Instant::now() + self.limits.stop_grace;
        while self.is_recording() && Instant::now() < deadline {
            finished.extend(self.poll(ffmpeg));
            std::thread::sleep(Duration::from_millis(100));
        }
        for a in &mut self.active {
            let _ = a.child.kill();
        }
        finished.extend(self.poll(ffmpeg));
        finished
    }

    /// Reap finished recordings, fail over to the next stream when a stream
    /// dies right away, and force-kill recordings that ignore a stop request.
    pub fn poll(&mut self, ffmpeg: Option<&Path>) -> Vec<Finished> {
        let lim = self.limits;
        let mut finished = Vec::new();
        let mut i = 0;
        while i < self.active.len() {
            let a = &mut self.active[i];
            let size_now = a.size_bytes();
            if size_now != a.last_size {
                a.last_size = size_now;
                a.last_growth = Instant::now();
            }
            if let Some(since) = a.stopping {
                if since.elapsed() > lim.stop_grace {
                    let _ = a.child.kill();
                }
            } else if let Some(d) = a.duration
                && a.started.elapsed() > d + lim.duration_grace
            {
                // Wall-clock end reached (the stream stalled): finish gracefully.
                a.stopping = Some(Instant::now());
                if let Some(stdin) = a.stdin.as_mut() {
                    let _ = stdin.write_all(b"q\n");
                    let _ = stdin.flush();
                }
            } else if size_now < 1024 && a.attempt_started.elapsed() > lim.start_stall {
                tracing::info!(channel = %a.channel_name, "recording: no data, giving up on this stream");
                let _ = a.child.kill();
            } else if size_now >= 1024 && a.last_growth.elapsed() > lim.mid_stall {
                tracing::info!(channel = %a.channel_name, "recording: stream stalled, stopping");
                let _ = a.child.kill();
            }
            let status = match a.child.try_wait() {
                Ok(Some(s)) => s,
                Ok(None) => {
                    i += 1;
                    continue;
                }
                Err(e) => {
                    tracing::warn!("waiting for ffmpeg failed: {e}");
                    i += 1;
                    continue;
                }
            };
            let size = a.size_bytes();
            let quick_fail = a.attempt_started.elapsed() < lim.fast_fail || size < 1024;
            // Failed start: try the next stream of the channel into the same file.
            if !status.success()
                && quick_fail
                && a.stopping.is_none()
                && let Some(ff) = ffmpeg
                && let Some(next) = a.queue.advance().cloned()
            {
                let remaining = a.duration.map(|d| d.saturating_sub(a.started.elapsed()));
                match spawn_ffmpeg(ff, &next, a.preset, &a.path, remaining, &a.log) {
                    Ok((child, stdin)) => {
                        tracing::info!(channel = %a.channel_name, "recording: stream failed, trying next");
                        a.child = child;
                        a.stdin = Some(stdin);
                        a.attempt_started = Instant::now();
                        i += 1;
                        continue;
                    }
                    Err(e) => tracing::warn!("could not start fallback stream: {e:#}"),
                }
            }
            let a = self.active.remove(i);
            let user_stopped = a.stopping.is_some();
            let log_line = last_log_line(&a.log);
            let _ = std::fs::remove_file(&a.log);
            let size = std::fs::metadata(&a.path).map(|m| m.len()).unwrap_or(0);
            let (ok, note) = if size < 1024 {
                let _ = std::fs::remove_file(&a.path);
                (
                    false,
                    nonempty(log_line, "no data received from the stream"),
                )
            } else if status.success() || user_stopped {
                (true, String::new())
            } else {
                (
                    true,
                    format!(
                        "ended early (stream dropped?), partial file kept. {}",
                        log_line
                    ),
                )
            };
            finished.push(Finished {
                channel_name: a.channel_name,
                preset: a.preset,
                path: a.path,
                started_at: a.started_at,
                size_bytes: if size < 1024 { 0 } else { size },
                ok,
                note,
                schedule_id: a.schedule_id,
            });
        }
        finished
    }
}

fn nonempty(s: String, fallback: &str) -> String {
    if s.trim().is_empty() {
        fallback.to_string()
    } else {
        s
    }
}

pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB * KB {
        format!("{:.0} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / KB / KB)
    } else {
        format!("{:.2} GB", b / KB / KB / KB)
    }
}

pub fn format_duration(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn stream() -> Stream {
        Stream {
            channel_id: "c".into(),
            url: "https://example.com/live.m3u8".into(),
            quality: None,
            referrer: None,
            user_agent: None,
        }
    }

    fn args(preset: Preset, d: Option<Duration>, s: &Stream) -> Vec<String> {
        build_args(s, preset, Path::new("/tmp/out.x"), d)
    }

    fn pos(a: &[String], what: &str) -> usize {
        a.iter().position(|x| x == what).unwrap()
    }

    #[test]
    fn preset_keys_round_trip() {
        for p in Preset::ALL {
            assert_eq!(Preset::from_key(p.key()), Some(p));
        }
        assert_eq!(Preset::from_key("nope"), None);
    }

    #[test]
    fn original_copies_without_reencoding() {
        let a = args(Preset::Original, None, &stream());
        assert!(a.windows(2).any(|w| w == ["-c", "copy"]));
        assert!(!a.contains(&"libx264".to_string()));
        assert!(!a.contains(&"-t".to_string()));
        assert_eq!(a.last().unwrap(), "/tmp/out.x");
    }

    #[test]
    fn reconnect_only_for_http_inputs() {
        let http = args(Preset::Original, None, &stream());
        assert!(http.contains(&"-reconnect".to_string()));
        let mut local = stream();
        local.url = "/tmp/source.ts".into();
        assert!(!args(Preset::Original, None, &local).contains(&"-reconnect".to_string()));
    }

    #[test]
    fn presets_get_progressively_smaller() {
        let crf = |p| {
            let a = args(p, None, &stream());
            a[pos(&a, "-crf") + 1].parse::<u32>().unwrap()
        };
        assert!(crf(Preset::Medium) < crf(Preset::Small));
        assert!(crf(Preset::Small) < crf(Preset::Tiny));
        let a = args(Preset::Tiny, None, &stream());
        assert!(a.iter().any(|x| x.starts_with("scale=-2:min(360")));
        assert_eq!(a[pos(&a, "-ac") + 1], "1"); // mono
    }

    #[test]
    fn audio_only_drops_video() {
        let a = args(Preset::AudioOnly, None, &stream());
        assert!(a.contains(&"-vn".to_string()));
        assert!(!a.contains(&"libx264".to_string()));
    }

    #[test]
    fn headers_and_duration_are_passed_as_input_and_output_options() {
        let mut s = stream();
        s.user_agent = Some("UA/1".into());
        s.referrer = Some("https://ref/".into());
        let a = args(Preset::Small, Some(Duration::from_secs(90)), &s);
        let i = pos(&a, "-i");
        assert!(pos(&a, "-user_agent") < i);
        assert_eq!(a[pos(&a, "-user_agent") + 1], "UA/1");
        assert!(pos(&a, "-referer") < i);
        assert!(pos(&a, "-t") > i); // output option
        assert_eq!(a[pos(&a, "-t") + 1], "90");
    }

    #[test]
    fn sanitize_makes_safe_file_names() {
        assert_eq!(sanitize("RTL 2 / HD: Live!"), "RTL_2__HD_Live");
        assert_eq!(sanitize("../../etc"), "etc");
        assert_eq!(sanitize("???"), "channel");
        assert_eq!(sanitize("Телевизија 1"), "Телевизија_1");
    }

    #[test]
    fn output_path_has_timestamp_extension_and_is_unique() {
        let dir = std::env::temp_dir().join(format!("iptv-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let t = Local.with_ymd_and_hms(2026, 11, 11, 20, 15, 30).unwrap();
        let p1 = output_path(&dir, "HRT 1", Preset::Small, t);
        assert!(p1.ends_with("HRT_1_20261111_201530.mp4"));
        std::fs::write(&p1, b"x").unwrap();
        let p2 = output_path(&dir, "HRT 1", Preset::Small, t);
        assert!(p2.ends_with("HRT_1_20261111_201530_2.mp4"));
        assert!(
            output_path(&dir, "HRT 1", Preset::Original, t)
                .extension()
                .unwrap()
                == "ts"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn find_ffmpeg_honours_custom_path() {
        assert_eq!(find_ffmpeg("/definitely/not/here"), None);
        let exe = std::env::current_exe().unwrap();
        assert_eq!(find_ffmpeg(exe.to_str().unwrap()), Some(exe));
    }

    #[test]
    fn formatting_helpers() {
        assert_eq!(format_size(512 * 1024), "512 KB");
        assert_eq!(format_size(3 * 1024 * 1024 + 512 * 1024), "3.5 MB");
        assert_eq!(format_duration(Duration::from_secs(3725)), "01:02:05");
    }

    /// End-to-end with a real ffmpeg and a locally generated source (no network):
    /// every preset must produce a playable file, and compressed presets must
    /// be smaller than the original copy. Skipped when ffmpeg is not installed.
    #[test]
    fn records_local_source_with_every_preset() {
        let Some(ffmpeg) = find_ffmpeg("") else {
            eprintln!("ffmpeg not installed, skipping");
            return;
        };
        let dir = std::env::temp_dir().join(format!("iptv-rec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("source.ts");
        let status = Command::new(&ffmpeg)
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=1280x720:rate=25",
            ])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000"])
            .args(["-t", "4", "-c:v", "libx264", "-b:v", "3M", "-c:a", "aac"])
            .arg(&src)
            .status()
            .unwrap();
        assert!(status.success(), "could not generate test source");

        let mut sizes = std::collections::HashMap::new();
        for preset in Preset::ALL {
            let mut rec = Recorder::default();
            let mut s = stream();
            s.url = src.to_string_lossy().into_owned();
            rec.start(
                &ffmpeg,
                RecordRequest {
                    channel_name: format!("Test {}", preset.key()),
                    streams: vec![s],
                    start_index: 0,
                    preset,
                    out_dir: dir.clone(),
                    duration: None,
                    schedule_id: None,
                },
            )
            .unwrap();
            let mut done = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(60);
            while done.is_empty() && Instant::now() < deadline {
                done = rec.poll(Some(&ffmpeg));
                std::thread::sleep(Duration::from_millis(100));
            }
            let f = done.pop().expect("recording finished");
            assert!(f.ok, "{:?} failed: {}", preset, f.note);
            assert!(f.path.extension().unwrap() == preset.extension());
            // The output must be readable by ffprobe/ffmpeg.
            let probe = Command::new(&ffmpeg)
                .args(["-v", "error", "-i"])
                .arg(&f.path)
                .args(["-f", "null", "-"])
                .status()
                .unwrap();
            assert!(probe.success(), "{:?} output is not decodable", preset);
            sizes.insert(preset.key(), f.size_bytes);
        }
        let _ = std::fs::remove_dir_all(&dir);
        eprintln!("sizes: {sizes:?}");
        assert!(sizes["small"] < sizes["original"]);
        assert!(sizes["tiny"] < sizes["small"]);
        assert!(sizes["audio"] < sizes["tiny"]);
    }

    /// Manual network test: records a real stream for a few seconds.
    ///   IPTV_LIVE_URL=https://.../x.m3u8 cargo test live_stream -- --ignored --nocapture
    #[test]
    #[ignore = "needs network and IPTV_LIVE_URL"]
    fn live_stream_records_with_each_compressed_preset() {
        let (Some(ffmpeg), Ok(url)) = (find_ffmpeg(""), std::env::var("IPTV_LIVE_URL")) else {
            panic!("set IPTV_LIVE_URL and install ffmpeg");
        };
        let dir = std::env::temp_dir().join(format!("iptv-live-{}", std::process::id()));
        let mut sizes = Vec::new();
        for preset in [
            Preset::Original,
            Preset::Medium,
            Preset::Tiny,
            Preset::AudioOnly,
        ] {
            let mut s = stream();
            s.url = url.clone();
            let mut rec = Recorder::default();
            rec.start(
                &ffmpeg,
                RecordRequest {
                    channel_name: "Live".into(),
                    streams: vec![s],
                    start_index: 0,
                    preset,
                    out_dir: dir.clone(),
                    duration: Some(Duration::from_secs(10)),
                    schedule_id: None,
                },
            )
            .unwrap();
            let mut done = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(90);
            while done.is_empty() && Instant::now() < deadline {
                done = rec.poll(Some(&ffmpeg));
                std::thread::sleep(Duration::from_millis(200));
            }
            let f = done.pop().expect("finished");
            assert!(f.ok, "{preset:?}: {}", f.note);
            sizes.push((preset.key(), f.size_bytes));
        }
        eprintln!("live sizes (10 s): {sizes:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A server that accepts connections and never sends a byte: ffmpeg would
    /// wait forever. The recorder must give up on each stream, fail over, and
    /// finally report failure without leaving a file or a process behind.
    #[test]
    fn stalled_streams_fail_over_and_are_given_up() {
        let Some(ffmpeg) = find_ffmpeg("") else {
            return;
        };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        // Detached on purpose: it blocks in accept() and ends with the test process.
        std::thread::spawn(move || {
            let mut conns = Vec::new();
            listener.set_nonblocking(false).unwrap();
            // Keep every connection open and silent.
            while let Ok((c, _)) = listener.accept() {
                conns.push(c);
                if conns.len() >= 8 {
                    break;
                }
            }
            std::thread::sleep(Duration::from_secs(12));
        });
        let dir = std::env::temp_dir().join(format!("iptv-stall-{}", std::process::id()));
        let mut rec = Recorder::with_limits(Limits {
            start_stall: Duration::from_secs(2),
            ..Limits::default()
        });
        let mk = |n: u32| {
            let mut s = stream();
            s.url = format!("http://127.0.0.1:{port}/{n}.m3u8");
            s
        };
        rec.start(
            &ffmpeg,
            RecordRequest {
                channel_name: "Stalled".into(),
                streams: vec![mk(1), mk(2)],
                start_index: 0,
                preset: Preset::Tiny,
                out_dir: dir.clone(),
                duration: Some(Duration::from_secs(5)),
                schedule_id: Some(9),
            },
        )
        .unwrap();
        let t0 = Instant::now();
        let mut done = Vec::new();
        while done.is_empty() && t0.elapsed() < Duration::from_secs(40) {
            done = rec.poll(Some(&ffmpeg));
            std::thread::sleep(Duration::from_millis(100));
        }
        let f = done
            .pop()
            .expect("recording must give up on stalled streams");
        assert!(!f.ok);
        assert!(!f.path.exists(), "empty file must be removed");
        assert!(!rec.is_recording());
        // Two streams x ~2 s stall each, plus process start-up: well under 30 s.
        assert!(
            t0.elapsed() < Duration::from_secs(30),
            "took {:?}",
            t0.elapsed()
        );
        drop(rec);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failing_stream_reports_failure_without_leaving_a_file() {
        let Some(ffmpeg) = find_ffmpeg("") else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("iptv-recfail-{}", std::process::id()));
        let mut rec = Recorder::default();
        let mut s = stream();
        s.url = "/definitely/missing/source.ts".into();
        rec.start(
            &ffmpeg,
            RecordRequest {
                channel_name: "Broken".into(),
                streams: vec![s.clone(), s],
                start_index: 0,
                preset: Preset::Tiny,
                out_dir: dir.clone(),
                duration: None,
                schedule_id: Some(7),
            },
        )
        .unwrap();
        let mut done = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(30);
        while done.is_empty() && Instant::now() < deadline {
            done = rec.poll(Some(&ffmpeg));
            std::thread::sleep(Duration::from_millis(100));
        }
        let f = done.pop().expect("finished");
        assert!(!f.ok);
        assert_eq!(f.schedule_id, Some(7));
        assert!(!f.path.exists());
        assert!(!f.note.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
