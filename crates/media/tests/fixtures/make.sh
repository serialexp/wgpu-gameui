#!/usr/bin/env bash
# Makes the test videos in this directory, and ffmpeg's decode of their
# pictures to check ours against (`*.framemd5`: one MD5 per frame, of its
# three planes one after the other). Run once; the output is committed, so the tests
# never need ffmpeg. Made with ffmpeg 6.1.1 and libx264 (short_sound.mp4
# with ffmpeg 9.0.2, on its own).
#
# `make.sh bench` instead makes the clip the decode measurement plays
# (`cargo test -p wgpu-gameui-media --release -- --ignored --nocapture`):
# ten seconds of 1080p30 High profile with AAC, too big to commit, written to
# crates/media/test_output/.
set -euo pipefail
cd "$(dirname "$0")"

quiet=(-hide_banner -loglevel error -y)

if [ "${1:-}" = bench ]; then
  mkdir -p ../../test_output
  # Moving noise over the test pattern, so the encoder spends its bits on
  # detail the way it would on camera footage, at about a phone's 1080p30
  # bitrate.
  ffmpeg "${quiet[@]}" \
    -f lavfi -i testsrc2=size=1920x1080:rate=30:duration=10 \
    -f lavfi -i 'sine=frequency=440:sample_rate=48000:duration=10' \
    -vf 'noise=alls=8:allf=t' \
    -c:v libx264 -profile:v high -pix_fmt yuv420p -preset medium \
    -b:v 8M -maxrate 10M -bufsize 16M -g 60 \
    -c:a aac -b:a 128k -ac 2 -movflags +faststart \
    ../../test_output/bench_1080p.mp4
  exit 0
fi

# High profile with B-frames (so decode order differs from display order), a
# keyframe every 15 frames, and a stereo AAC-LC track: 440 Hz on the left,
# 660 Hz on the right.
ffmpeg "${quiet[@]}" \
  -f lavfi -i testsrc2=size=320x180:rate=30:duration=2 \
  -f lavfi -i 'aevalsrc=0.5*sin(440*2*PI*t)|0.5*sin(660*2*PI*t):s=48000:d=2' \
  -c:v libx264 -profile:v high -pix_fmt yuv420p -bf 3 -g 15 -keyint_min 15 \
  -sc_threshold 0 -c:a aac -b:a 96k -movflags +faststart -map_metadata -1 \
  -fflags +bitexact -flags:v +bitexact -flags:a +bitexact \
  h264_aac.mp4

# Baseline profile, no sound, at a standard-definition size, but saying it
# uses BT.709 and the full range: what players would assume for its size is
# neither, so it shows whether the stream's own settings are read.
ffmpeg "${quiet[@]}" \
  -f lavfi -i testsrc=size=176x144:rate=25:duration=1 \
  -c:v libx264 -profile:v baseline -pix_fmt yuv420p -g 10 -sc_threshold 0 \
  -color_range pc -colorspace bt709 -color_primaries bt709 -color_trc bt709 \
  -movflags +faststart -map_metadata -1 -fflags +bitexact -flags:v +bitexact \
  baseline_full_range.mp4

# Sound from the start and pictures only from half a second (the video
# track's edit list starts with an empty edit), ending half a second before
# the sound: sound queued before its picture after a seek, and sound playing
# on after the last picture. The pictures are encoded first: `-itsoffset`
# only moves a file's stream, not one lavfi makes.
late=$(mktemp -d)
ffmpeg "${quiet[@]}" \
  -f lavfi -i testsrc=size=176x144:rate=25:duration=0.5 \
  -c:v libx264 -profile:v baseline -pix_fmt yuv420p -g 5 -sc_threshold 0 \
  -map_metadata -1 -fflags +bitexact -flags:v +bitexact \
  "$late/video.mp4"
ffmpeg "${quiet[@]}" \
  -itsoffset 0.5 -i "$late/video.mp4" \
  -f lavfi -i 'sine=frequency=440:sample_rate=48000:duration=1.5' \
  -map 0:v -map 1:a -c:v copy -c:a aac -b:a 64k -ac 1 \
  -movflags +faststart -map_metadata -1 -fflags +bitexact -flags:a +bitexact \
  late_video.mp4
rm -r "$late"

# Sound that ends long before the pictures: a third of a second of it under
# a second and a half of video, further than the worker decodes pictures
# ahead, so playing it shows the clock isn't left waiting for sound that
# won't come.
ffmpeg "${quiet[@]}" \
  -f lavfi -i testsrc=size=176x144:rate=25:duration=1.5 \
  -f lavfi -i 'sine=frequency=440:sample_rate=48000:duration=0.3' \
  -c:v libx264 -profile:v baseline -pix_fmt yuv420p -g 10 -sc_threshold 0 \
  -c:a aac -b:a 64k -ac 1 -movflags +faststart -map_metadata -1 \
  -fflags +bitexact -flags:v +bitexact -flags:a +bitexact \
  short_sound.mp4

# Sound alone: one second of mono AAC-LC in an M4A.
ffmpeg "${quiet[@]}" \
  -f lavfi -i 'sine=frequency=440:sample_rate=44100:duration=1' \
  -c:a aac -b:a 64k -ac 1 -movflags +faststart -map_metadata -1 \
  -fflags +bitexact -flags:a +bitexact \
  aac_only.m4a

# What the decoder can't play: interlaced coding and 10-bit samples.
ffmpeg "${quiet[@]}" \
  -f lavfi -i testsrc=size=176x144:rate=25:duration=0.2 \
  -c:v libx264 -pix_fmt yuv420p -flags +ilme+ildct -x264-params interlaced=1 \
  -movflags +faststart -map_metadata -1 -fflags +bitexact \
  interlaced.mp4
ffmpeg "${quiet[@]}" \
  -f lavfi -i testsrc=size=176x144:rate=25:duration=0.2 \
  -c:v libx264 -profile:v high10 -pix_fmt yuv420p10le \
  -movflags +faststart -map_metadata -1 -fflags +bitexact \
  high10.mp4

# Codecs the player doesn't decode: H.265 video, and H.264 with MP3 sound.
ffmpeg "${quiet[@]}" \
  -f lavfi -i testsrc=size=176x144:rate=25:duration=0.2 \
  -c:v libx265 -tag:v hvc1 -pix_fmt yuv420p -x265-params log-level=error \
  -movflags +faststart -map_metadata -1 -fflags +bitexact \
  hevc.mp4
ffmpeg "${quiet[@]}" \
  -f lavfi -i testsrc=size=176x144:rate=25:duration=0.2 \
  -f lavfi -i 'sine=frequency=440:sample_rate=44100:duration=0.2' \
  -c:v libx264 -pix_fmt yuv420p -c:a libmp3lame \
  -movflags +faststart -map_metadata -1 -fflags +bitexact \
  mp3_audio.mp4

# The decoder's own planes, unconverted: asking for yuv420p would rescale the
# full-range video's codes.
for video in h264_aac baseline_full_range; do
  ffmpeg "${quiet[@]}" -i "$video.mp4" -map 0:v -f framemd5 "$video.framemd5"
done
