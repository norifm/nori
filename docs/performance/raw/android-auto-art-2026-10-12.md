# Android Auto artwork, October 12, 2026

Applies to Android 0.6.0-beta.3. Android 14 x86_64 emulator on Arch Linux, local Navidrome.

The car's artwork provider draws a four-cover mosaic using the phone's shared widget painter.
Filtering out failed cover loads changed the number of pictures from four to three. The painter
then drew only the first picture across the entire tile, losing two other available covers.
The provider and widgets now pass all four positions, including unavailable pictures; the painter
leaves those quadrants in the tile's background color.

The regression opens the actual content-provider URI twice: first with four distinct fixture covers,
then with the fourth unavailable. Both JPEGs are decoded on Android. It compares a 50 × 50 pixel
area inside each of the three available quadrants, across all RGB channels.

| Render behavior | Top-left mean error | Top-right mean error | Bottom-left mean error |
| --- | ---: | ---: | ---: |
| Filter unavailable pictures | 0.0 | 43.67 | 32.92 |
| Keep the four positions | 0.0 | 0.0 | 0.0 |

The debug APK and app JVM tests passed. Smoke passed 34 checks. Artwork, partially downloaded albums,
Favorites through the real media3 browser, offline playback, and the phone's mix pages passed 26 checks.
This exercises the content provider and media browser; it does not claim a rendered Android Auto
head-unit check. The installed Android Auto package on this emulator is a settings stub.
