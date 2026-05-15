#[cfg(feature = "realtime-webrtc")]
pub(crate) use codex_realtime_webrtc::RealtimeWebrtcEvent;
#[cfg(feature = "realtime-webrtc")]
pub(crate) use codex_realtime_webrtc::RealtimeWebrtcSession;
#[cfg(feature = "realtime-webrtc")]
pub(crate) use codex_realtime_webrtc::RealtimeWebrtcSessionHandle;

#[cfg(not(feature = "realtime-webrtc"))]
use std::fmt;
#[cfg(not(feature = "realtime-webrtc"))]
use std::sync::Arc;
#[cfg(not(feature = "realtime-webrtc"))]
use std::sync::atomic::AtomicU16;
#[cfg(not(feature = "realtime-webrtc"))]
use std::sync::mpsc;

#[cfg(not(feature = "realtime-webrtc"))]
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum RealtimeWebrtcEvent {
    Connected,
    LocalAudioLevel(u16),
    Closed,
    Failed(String),
}

#[cfg(not(feature = "realtime-webrtc"))]
#[allow(dead_code)]
pub(crate) struct StartedRealtimeWebrtcSession {
    pub(crate) offer_sdp: String,
    pub(crate) handle: RealtimeWebrtcSessionHandle,
    pub(crate) events: mpsc::Receiver<RealtimeWebrtcEvent>,
}

#[cfg(not(feature = "realtime-webrtc"))]
#[derive(Clone)]
pub(crate) struct RealtimeWebrtcSessionHandle {
    local_audio_peak: Arc<AtomicU16>,
}

#[cfg(not(feature = "realtime-webrtc"))]
impl fmt::Debug for RealtimeWebrtcSessionHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RealtimeWebrtcSessionHandle")
            .finish_non_exhaustive()
    }
}

#[cfg(not(feature = "realtime-webrtc"))]
impl RealtimeWebrtcSessionHandle {
    pub(crate) fn apply_answer_sdp(&self, _answer_sdp: String) -> Result<(), String> {
        Err("realtime WebRTC is unavailable in this build".to_string())
    }

    pub(crate) fn close(&self) {}

    pub(crate) fn local_audio_peak(&self) -> Arc<AtomicU16> {
        self.local_audio_peak.clone()
    }
}

#[cfg(not(feature = "realtime-webrtc"))]
#[allow(dead_code)]
pub(crate) struct RealtimeWebrtcSession;

#[cfg(not(feature = "realtime-webrtc"))]
impl RealtimeWebrtcSession {
    #[allow(dead_code)]
    pub(crate) fn start() -> Result<StartedRealtimeWebrtcSession, String> {
        Err("realtime WebRTC is unavailable in this build".to_string())
    }
}
