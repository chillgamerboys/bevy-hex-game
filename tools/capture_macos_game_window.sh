#!/bin/zsh
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  print -u2 "native Hex Game capture requires macOS"
  exit 2
fi
if (( $# != 2 )); then
  print -u2 "usage: $0 OUTPUT.png CHECKPOINT"
  exit 2
fi

output_path="$1"
checkpoint="$2"
window_title="${HEX_WALK_WINDOW_TITLE:-Hex Game}"

window_record="$(/usr/bin/swift - "$window_title" <<'SWIFT'
import CoreGraphics
import Foundation

let wanted = CommandLine.arguments[1]
let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
guard let records = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] else {
    exit(3)
}
for record in records {
    guard let layer = record[kCGWindowLayer as String] as? Int, layer == 0,
          let number = record[kCGWindowNumber as String] as? Int else {
        continue
    }
    let boundsDictionary = record[kCGWindowBounds as String] as! CFDictionary
    guard let bounds = CGRect(dictionaryRepresentation: boundsDictionary) else {
        continue
    }
    let name = record[kCGWindowName as String] as? String ?? ""
    let owner = record[kCGWindowOwnerName as String] as? String ?? ""
    if name == wanted && owner == "hex_game" {
        print("\(number)\t\(bounds.origin.x)\t\(bounds.origin.y)\t\(bounds.width)\t\(bounds.height)\t\(owner)\t\(name)")
        exit(0)
    }
}
exit(4)
SWIFT
)" || {
  print -u2 "could not find an on-screen Hex Game window named '$window_title'"
  exit 3
}

IFS=$'\t' read -r window_id window_x window_y window_width window_height owner_name actual_title <<< "$window_record"
mkdir -p "${output_path:h}"
/usr/sbin/screencapture -x -o -l "$window_id" "$output_path"

brightest_rgb="$(/usr/bin/swift - "$output_path" <<'SWIFT'
import CoreGraphics
import Foundation
import ImageIO

let url = URL(fileURLWithPath: CommandLine.arguments[1]) as CFURL
guard let source = CGImageSourceCreateWithURL(url, nil),
      let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else {
    exit(3)
}
let width = 64
let height = 64
var pixels = [UInt8](repeating: 0, count: width * height * 4)
let drew = pixels.withUnsafeMutableBytes { storage -> Bool in
    guard let context = CGContext(
        data: storage.baseAddress,
        width: width,
        height: height,
        bitsPerComponent: 8,
        bytesPerRow: width * 4,
        space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ) else {
        return false
    }
    context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
    return true
}
guard drew else {
    exit(4)
}
var brightest: UInt8 = 0
for index in stride(from: 0, to: pixels.count, by: 4) {
    brightest = max(brightest, pixels[index], pixels[index + 1], pixels[index + 2])
}
print(brightest)
exit(brightest > 4 ? 0 : 5)
SWIFT
)" || {
  print -u2 "native capture is empty or black at checkpoint '$checkpoint'"
  exit 4
}

capture_width="$(/usr/bin/sips -g pixelWidth "$output_path" | /usr/bin/awk '/pixelWidth:/ {print $2}')"
capture_height="$(/usr/bin/sips -g pixelHeight "$output_path" | /usr/bin/awk '/pixelHeight:/ {print $2}')"
commit_sha="$(git rev-parse HEAD)"
metadata_path="${output_path%.png}.metadata.tsv"
{
  print "checkpoint\tcommit_sha\twindow_id\twindow_bounds_points\tcapture_pixels\tbrightest_rgb\tbevy_physical\tbevy_logical\tos_scale\tui_scale\tviewport\tmode\towner\ttitle"
  print "${checkpoint}\t${commit_sha}\t${window_id}\t${window_x},${window_y},${window_width},${window_height}\t${capture_width}x${capture_height}\t${brightest_rgb}\t${HEX_WALK_WINDOW_PHYSICAL:-unknown}\t${HEX_WALK_WINDOW_LOGICAL:-unknown}\t${HEX_WALK_WINDOW_SCALE_FACTOR:-unknown}\t${HEX_WALK_UI_SCALE:-unknown}\t${HEX_WALK_VIEWPORT_CLASS:-unknown}\t${HEX_WALK_WINDOW_MODE:-unknown}\t${owner_name}\t${actual_title}"
} > "$metadata_path"
