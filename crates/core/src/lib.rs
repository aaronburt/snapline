#![doc = "Domain types, pure logic and port traits for Snapline. This crate owns ids and newtypes, geometry, images, ports, the action catalogue, chords, the hotkey plan, naming, link formats, the overlay and recording state machines, annotation, the software rasteriser, workflow running, presets, FFmpeg arguments, stitching, ruler maths, colour conversions, QR wrappers and combiner layout. It performs no OS calls and contains no unsafe code."]
#![forbid(unsafe_code)]

pub mod actions;
pub mod chord;
pub mod ffmpeg_args;
pub mod geometry;
pub mod hotkey_plan;
pub mod ids;
pub mod image;
pub mod link_format;
pub mod naming;
pub mod overlay;
pub mod ports;
pub mod recording;
pub mod secret;
