Synthetic six-second video and sine-wave audio for offline integration tests. No third-party media. Generated with:

```sh
ffmpeg -f lavfi -i testsrc2=size=320x180:rate=24 -f lavfi -i sine=frequency=440:sample_rate=44100 -t 6 -c:v libx264 -preset veryfast -b:v 300k -g 48 -c:a aac -b:a 64k -seg_duration 2 -use_template 1 -use_timeline 1 -f dash manifest.mpd
```
