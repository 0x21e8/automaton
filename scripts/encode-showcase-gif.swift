import Foundation
import ImageIO
import UniformTypeIdentifiers

guard CommandLine.arguments.count == 3 else {
  fputs("usage: encode-showcase-gif.swift <frames-dir> <output.gif>\n", stderr)
  exit(64)
}

let framesDirectory = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
let output = URL(fileURLWithPath: CommandLine.arguments[2])
let files = try FileManager.default.contentsOfDirectory(
  at: framesDirectory,
  includingPropertiesForKeys: nil
).filter { $0.pathExtension.lowercased() == "png" }.sorted { $0.lastPathComponent < $1.lastPathComponent }

guard !files.isEmpty else {
  fputs("no PNG frames found\n", stderr)
  exit(65)
}

func blackPixelFraction(_ image: CGImage) -> Double {
  let width = 120
  let height = 68
  var pixels = [UInt8](repeating: 0, count: width * height * 4)
  guard let context = CGContext(
    data: &pixels,
    width: width,
    height: height,
    bitsPerComponent: 8,
    bytesPerRow: width * 4,
    space: CGColorSpaceCreateDeviceRGB(),
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
  ) else {
    return 1
  }
  context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))

  var blackPixels = 0
  for offset in stride(from: 0, to: pixels.count, by: 4) {
    if pixels[offset] < 5 && pixels[offset + 1] < 5 && pixels[offset + 2] < 5 {
      blackPixels += 1
    }
  }
  return Double(blackPixels) / Double(width * height)
}

let frames: [(URL, CGImage)] = files.compactMap { file in
  guard let source = CGImageSourceCreateWithURL(file as CFURL, nil),
        let image = CGImageSourceCreateThumbnailAtIndex(source, 0, [
          kCGImageSourceCreateThumbnailFromImageAlways: true,
          kCGImageSourceThumbnailMaxPixelSize: 900,
          kCGImageSourceCreateThumbnailWithTransform: true
        ] as CFDictionary) else {
    return nil
  }
  return (file, image)
}.filter { file, image in
  let isClean = blackPixelFraction(image) < 0.08
  if !isClean {
    print("Skipping corrupted frame \(file.lastPathComponent)")
  }
  return isClean
}

guard !frames.isEmpty else {
  fputs("all PNG frames were rejected as corrupted\n", stderr)
  exit(65)
}

guard let destination = CGImageDestinationCreateWithURL(
  output as CFURL,
  UTType.gif.identifier as CFString,
  frames.count,
  nil
) else {
  fputs("could not create GIF destination\n", stderr)
  exit(66)
}

CGImageDestinationSetProperties(destination, [
  kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFLoopCount: 0]
] as CFDictionary)

for (_, image) in frames {
  CGImageDestinationAddImage(destination, image, [
    kCGImagePropertyGIFDictionary: [
      kCGImagePropertyGIFDelayTime: 0.10,
      kCGImagePropertyGIFUnclampedDelayTime: 0.10
    ]
  ] as CFDictionary)
}

guard CGImageDestinationFinalize(destination) else {
  fputs("could not finalize GIF\n", stderr)
  exit(68)
}
