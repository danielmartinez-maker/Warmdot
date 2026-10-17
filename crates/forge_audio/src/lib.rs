//! Forge-owned audio playback with Kira kept behind the adapter boundary.

use std::{error::Error, fmt};

use forge_assets::{AssetCatalog, AssetError};
use forge_core::Guid;
use kira::{
    AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Tween,
    sound::{
        FromFileError, PlaybackState,
        static_sound::{StaticSoundData, StaticSoundHandle},
        streaming::{StreamingSoundData, StreamingSoundHandle},
    },
    track::{TrackBuilder, TrackHandle},
};

const MAX_STATIC_SOUND_BYTES: u64 = 32 * 1024 * 1024;
const MIN_BUS_DECIBELS: f32 = -60.0;
const MAX_BUS_DECIBELS: f32 = 6.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioBus {
    Master,
    Music,
    Sfx,
    Ui,
}

impl AudioBus {
    pub fn parse(value: &str) -> Result<Self, AudioError> {
        match value {
            "master" => Ok(Self::Master),
            "music" => Ok(Self::Music),
            "sfx" => Ok(Self::Sfx),
            "ui" => Ok(Self::Ui),
            _ => Err(AudioError::InvalidBus(value.to_owned())),
        }
    }
}

#[derive(Debug)]
pub enum AudioError {
    Asset(AssetError),
    Device(String),
    Decode(String),
    Playback(String),
    InvalidBus(String),
    InvalidVolume(f32),
    StaticSoundTooLarge(u64),
    MusicBusForEffect,
}

impl fmt::Display for AudioError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Asset(error) => write!(formatter, "{error}"),
            Self::Device(message) => write!(formatter, "audio device unavailable: {message}"),
            Self::Decode(message) => write!(formatter, "audio decode failed: {message}"),
            Self::Playback(message) => write!(formatter, "audio playback failed: {message}"),
            Self::InvalidBus(bus) => write!(formatter, "unknown audio bus {bus:?}"),
            Self::InvalidVolume(volume) => write!(
                formatter,
                "audio bus volume {volume} dB must be finite and between {MIN_BUS_DECIBELS} and {MAX_BUS_DECIBELS} dB"
            ),
            Self::StaticSoundTooLarge(size) => write!(
                formatter,
                "sound effect is {size} bytes; maximum static sound size is {MAX_STATIC_SOUND_BYTES}"
            ),
            Self::MusicBusForEffect => formatter.write_str("use play_music for the music bus"),
        }
    }
}

impl Error for AudioError {}

impl From<AssetError> for AudioError {
    fn from(error: AssetError) -> Self {
        Self::Asset(error)
    }
}

pub struct AudioEngine {
    manager: AudioManager<DefaultBackend>,
    music_track: TrackHandle,
    sfx_track: TrackHandle,
    ui_track: TrackHandle,
    music: Option<StreamingSoundHandle<FromFileError>>,
    effects: Vec<StaticSoundHandle>,
}

impl AudioEngine {
    pub fn new() -> Result<Self, AudioError> {
        let mut manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .map_err(|error| AudioError::Device(error.to_string()))?;
        let music_track = manager
            .add_sub_track(TrackBuilder::new())
            .map_err(|error| AudioError::Playback(error.to_string()))?;
        let sfx_track = manager
            .add_sub_track(TrackBuilder::new())
            .map_err(|error| AudioError::Playback(error.to_string()))?;
        let ui_track = manager
            .add_sub_track(TrackBuilder::new())
            .map_err(|error| AudioError::Playback(error.to_string()))?;

        Ok(Self {
            manager,
            music_track,
            sfx_track,
            ui_track,
            music: None,
            effects: Vec::new(),
        })
    }

    pub fn play_effect(
        &mut self,
        assets: &AssetCatalog,
        asset_guid: Guid,
        bus: AudioBus,
    ) -> Result<(), AudioError> {
        if bus == AudioBus::Music {
            return Err(AudioError::MusicBusForEffect);
        }
        let asset = assets.load_audio(asset_guid)?;
        if asset.source_bytes > MAX_STATIC_SOUND_BYTES {
            return Err(AudioError::StaticSoundTooLarge(asset.source_bytes));
        }
        let sound = StaticSoundData::from_file(&asset.path)
            .map_err(|error| AudioError::Decode(error.to_string()))?;
        let handle = match bus {
            AudioBus::Master => self.manager.main_track().play(sound),
            AudioBus::Sfx => self.sfx_track.play(sound),
            AudioBus::Ui => self.ui_track.play(sound),
            AudioBus::Music => unreachable!("music bus was rejected above"),
        }
        .map_err(|error| AudioError::Playback(error.to_string()))?;
        self.effects.push(handle);
        self.effects
            .retain(|effect| effect.state() != PlaybackState::Stopped);
        Ok(())
    }

    pub fn play_music(
        &mut self,
        assets: &AssetCatalog,
        asset_guid: Guid,
        looping: bool,
    ) -> Result<(), AudioError> {
        let asset = assets.load_audio(asset_guid)?;
        let mut sound = StreamingSoundData::from_file(&asset.path)
            .map_err(|error| AudioError::Decode(error.to_string()))?;
        if looping {
            sound = sound.loop_region(0.0..);
        }
        if let Some(mut previous) = self.music.take() {
            previous.stop(Tween::default());
        }
        self.music = Some(
            self.music_track
                .play(sound)
                .map_err(|error| AudioError::Playback(error.to_string()))?,
        );
        Ok(())
    }

    pub fn set_bus_volume(&mut self, bus: AudioBus, decibels: f32) -> Result<(), AudioError> {
        if !decibels.is_finite() || !(MIN_BUS_DECIBELS..=MAX_BUS_DECIBELS).contains(&decibels) {
            return Err(AudioError::InvalidVolume(decibels));
        }
        let volume = Decibels(decibels);
        match bus {
            AudioBus::Master => self
                .manager
                .main_track()
                .set_volume(volume, Tween::default()),
            AudioBus::Music => self.music_track.set_volume(volume, Tween::default()),
            AudioBus::Sfx => self.sfx_track.set_volume(volume, Tween::default()),
            AudioBus::Ui => self.ui_track.set_volume(volume, Tween::default()),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_named_forge_buses() {
        assert_eq!(AudioBus::parse("master").unwrap(), AudioBus::Master);
        assert_eq!(AudioBus::parse("music").unwrap(), AudioBus::Music);
        assert_eq!(AudioBus::parse("sfx").unwrap(), AudioBus::Sfx);
        assert_eq!(AudioBus::parse("ui").unwrap(), AudioBus::Ui);
        assert!(AudioBus::parse("native-track").is_err());
    }

    #[test]
    fn bus_volume_range_is_explicit() {
        assert!(
            AudioError::InvalidVolume(f32::NAN)
                .to_string()
                .contains("finite")
        );
        assert!(AudioError::InvalidVolume(-61.0).to_string().contains("-60"));
        assert!(AudioError::InvalidVolume(7.0).to_string().contains("6"));
    }


    #[test]
    fn decodes_pcm_wav_assets_used_by_action_arena() {
        let audio_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/action_arena/Assets/Audio");
        let click = audio_dir.join("click.wav");
        StaticSoundData::from_file(&click).unwrap_or_else(|error| {
            panic!("Action Arena PCM click WAV failed to decode: {error}")
        });

        let music = audio_dir.join("arena-music.wav");
        StreamingSoundData::from_file(&music).unwrap_or_else(|error| {
            panic!("Action Arena PCM music WAV failed to decode: {error}")
        });
    }
}
