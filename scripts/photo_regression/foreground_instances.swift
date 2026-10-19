// Original image-only Vision adapter. Platform framework; no third-party source.
import Foundation
import Vision
import CoreImage
import ImageIO
let args = CommandLine.arguments
if args.count != 3 { fatalError("foreground_instances PHOTOS OUTPUT") }
let source = URL(fileURLWithPath: args[1], isDirectory: true)
let output = URL(fileURLWithPath: args[2], isDirectory: true)
try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
let context = CIContext(options: [.useSoftwareRenderer: false])
let files = try FileManager.default.contentsOfDirectory(at: source, includingPropertiesForKeys: nil)
    .filter { ["jpg", "jpeg", "png"].contains($0.pathExtension.lowercased()) }
    .sorted { $0.lastPathComponent < $1.lastPathComponent }
for file in files {
    do { try autoreleasepool {
        guard let photo = CIImage(contentsOf: file) else { return }
        let full = photo.extent
        for margin in [0.0, 0.18, 0.30] {
            let crop = full.insetBy(dx: full.width * margin, dy: full.height * margin)
            let patch = photo.cropped(to: crop).transformed(by: CGAffineTransform(translationX: -crop.minX, y: -crop.minY))
            guard let image = context.createCGImage(patch, from: patch.extent) else { continue }
            let handler = VNImageRequestHandler(cgImage: image, options: [:])
            let request = VNGenerateForegroundInstanceMaskRequest()
            try handler.perform([request])
            guard let observation = request.results?.first, !observation.allInstances.isEmpty else { continue }
            for instance in observation.allInstances {
                let mask = try observation.generateScaledMaskForImage(forInstances: IndexSet(integer: instance), from: handler)
                let image = CIImage(cvPixelBuffer: mask).transformed(by: CGAffineTransform(translationX: crop.minX, y: crop.minY))
                    .composited(over: CIImage(color: .black).cropped(to: full))
                try context.writePNGRepresentation(of: image, to: output.appendingPathComponent(file.lastPathComponent + ".instance-\(instance).png"), format: .L8, colorSpace: CGColorSpaceCreateDeviceGray())
            }
            break
        }
        print(file.lastPathComponent); fflush(stdout)
    }} catch { print("Vision rejected \(file.lastPathComponent): \(error)"); fflush(stdout) }
}
