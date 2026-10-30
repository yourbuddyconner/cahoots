import AVFoundation
import SwiftUI

/// The camera preview. Up to ten frames a second go to the phone crate's QR decoder as
/// grayscale (the luma plane), and each code read is handed to `onCode` on the main thread.
/// The same code read twice in a row is handed over once.
struct CameraScanner: UIViewRepresentable {
    let core: Core
    let onCode: (String) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(core: core) }

    func makeUIView(context: Context) -> PreviewView {
        let view = PreviewView()
        context.coordinator.onCode = onCode
        context.coordinator.attach(to: view)
        return view
    }

    func updateUIView(_ view: PreviewView, context: Context) {
        context.coordinator.onCode = onCode
    }

    static func dismantleUIView(_ view: PreviewView, coordinator: Coordinator) {
        coordinator.stop()
    }

    final class PreviewView: UIView {
        override class var layerClass: AnyClass { AVCaptureVideoPreviewLayer.self }
        var preview: AVCaptureVideoPreviewLayer { layer as! AVCaptureVideoPreviewLayer }
        let message = UILabel()

        override init(frame: CGRect) {
            super.init(frame: frame)
            backgroundColor = .black
            preview.videoGravity = .resizeAspectFill
            message.textColor = .white
            message.numberOfLines = 0
            message.textAlignment = .center
            message.font = .preferredFont(forTextStyle: .footnote)
            message.translatesAutoresizingMaskIntoConstraints = false
            addSubview(message)
            NSLayoutConstraint.activate([
                message.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 16),
                message.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -16),
                message.centerYAnchor.constraint(equalTo: centerYAnchor),
            ])
        }

        required init?(coder: NSCoder) { fatalError("not used") }
    }

    final class Coordinator: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate, @unchecked Sendable {
        let core: Core
        var onCode: (String) -> Void = { _ in }
        private let session = AVCaptureSession()
        private let frames = DispatchQueue(label: "network.cahoots.camera")
        private var last = Date.distantPast
        private var lastText: String?

        init(core: Core) { self.core = core }

        func attach(to view: PreviewView) {
            view.preview.session = session
            guard AVCaptureDevice.default(for: .video) != nil else {
                view.message.text = "No camera here. Paste a code instead."
                return
            }
            switch AVCaptureDevice.authorizationStatus(for: .video) {
            case .authorized:
                start()
            case .notDetermined:
                AVCaptureDevice.requestAccess(for: .video) { ok in
                    DispatchQueue.main.async {
                        if ok { self.start() } else { view.message.text = Self.off }
                    }
                }
            default:
                view.message.text = Self.off
            }
        }

        static let off = "Camera access is off for Cahoots. Allow it in Settings, or paste a code instead."

        private func start() {
            frames.async { [self] in
                guard let device = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back)
                        ?? AVCaptureDevice.default(for: .video),
                      let input = try? AVCaptureDeviceInput(device: device) else { return }
                session.beginConfiguration()
                session.sessionPreset = .hd1280x720
                if session.canAddInput(input) { session.addInput(input) }
                let output = AVCaptureVideoDataOutput()
                output.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String:
                                        kCVPixelFormatType_420YpCbCr8BiPlanarFullRange]
                output.alwaysDiscardsLateVideoFrames = true
                output.setSampleBufferDelegate(self, queue: frames)
                if session.canAddOutput(output) { session.addOutput(output) }
                session.commitConfiguration()
                session.startRunning()
            }
        }

        func stop() {
            frames.async { [session] in session.stopRunning() }
        }

        func captureOutput(_ output: AVCaptureOutput, didOutput sample: CMSampleBuffer,
                           from connection: AVCaptureConnection) {
            let now = Date()
            guard now.timeIntervalSince(last) >= 0.1, let pixels = CMSampleBufferGetImageBuffer(sample) else { return }
            last = now
            CVPixelBufferLockBaseAddress(pixels, .readOnly)
            defer { CVPixelBufferUnlockBaseAddress(pixels, .readOnly) }
            let w = CVPixelBufferGetWidthOfPlane(pixels, 0)
            let h = CVPixelBufferGetHeightOfPlane(pixels, 0)
            let stride = CVPixelBufferGetBytesPerRowOfPlane(pixels, 0)
            guard let base = CVPixelBufferGetBaseAddressOfPlane(pixels, 0) else { return }
            var gray = Data(count: w * h)
            gray.withUnsafeMutableBytes { dst in
                guard let d = dst.baseAddress else { return }
                for y in 0..<h {
                    memcpy(d + y * w, base + y * stride, w)
                }
            }
            guard let text = core.decodeFrame(width: w, height: h, gray: gray), text != lastText else { return }
            lastText = text
            DispatchQueue.main.async { self.onCode(text) }
        }
    }
}
