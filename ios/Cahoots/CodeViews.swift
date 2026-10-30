import SwiftUI

/// A QR code drawn from the phone crate's modules, with a white quiet zone.
struct QRCodeView: View {
    let matrix: QrMatrix
    var caption: String?

    var body: some View {
        VStack(spacing: 8) {
            Canvas { ctx, size in
                let n = Int(matrix.size)
                let quiet = 4
                let side = min(size.width, size.height)
                let cell = side / CGFloat(n + 2 * quiet)
                ctx.fill(Path(CGRect(x: 0, y: 0, width: side, height: side)), with: .color(.white))
                var dark = Path()
                for y in 0..<n {
                    for x in 0..<n where matrix.bits[y * n + x] {
                        dark.addRect(CGRect(x: CGFloat(x + quiet) * cell, y: CGFloat(y + quiet) * cell,
                                            width: cell + 0.5, height: cell + 0.5))
                    }
                }
                ctx.fill(dark, with: .color(.black))
            }
            .aspectRatio(1, contentMode: .fit)
            .clipShape(RoundedRectangle(cornerRadius: 12))
            .accessibilityElement()
            .accessibilityLabel("Code")
            .accessibilityIdentifier("code")
            if let caption {
                Text(caption).font(.footnote).foregroundStyle(.secondary)
            }
        }
    }
}

/// Codes shown one after another at ten a second, as the browser app's animations are.
struct CodeAnimation: View {
    let frames: [QrMatrix]
    var caption: String?

    var body: some View {
        if frames.isEmpty {
            ProgressView().frame(maxWidth: .infinity, minHeight: 200)
        } else {
            TimelineView(.periodic(from: .now, by: 0.1)) { context in
                let i = Int(context.date.timeIntervalSinceReferenceDate * 10) % frames.count
                QRCodeView(matrix: frames[i], caption: caption)
            }
        }
    }
}

/// The camera, and below it a field to paste a code: for the simulator, for a phone whose camera
/// is off, and for anyone who prefers text.
struct ScanPanel: View {
    let core: Core
    var hint = "Fill the frame with the code"
    var placeholder = "cahoots:..."
    let onCode: (String, _ pasted: Bool) -> Void
    @State private var pasted = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            CameraScanner(core: core) { onCode($0, false) }
                .frame(height: 300)
                .clipShape(RoundedRectangle(cornerRadius: 12))
            Text(hint).font(.footnote).foregroundStyle(.secondary)
            Text("No camera? Paste a code").font(.subheadline.weight(.semibold))
            HStack {
                TextField(placeholder, text: $pasted, axis: .vertical)
                    .lineLimit(1...4)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .font(.footnote.monospaced())
                    .textFieldStyle(.roundedBorder)
                    .accessibilityIdentifier("paste-code")
                Button("Use") {
                    let t = pasted.trimmingCharacters(in: .whitespacesAndNewlines)
                    guard !t.isEmpty else { return }
                    pasted = ""
                    onCode(t, true)
                }
                .buttonStyle(.bordered)
                .accessibilityIdentifier("use-code")
            }
        }
    }
}

/// A line of status under a screen, in red when it is a problem.
struct StatusLine: View {
    let text: String?
    var problem = true

    var body: some View {
        if let text, !text.isEmpty {
            Text(text)
                .font(.footnote)
                .foregroundStyle(problem ? Color.red : Color.secondary)
                .accessibilityIdentifier("status")
        }
    }
}

/// Shown while the seat key waits for Face ID.
struct FaceIDBanner: View {
    var body: some View {
        HStack(spacing: 10) {
            ProgressView()
            Text("Confirm with Face ID")
        }
        .padding(12)
        .frame(maxWidth: .infinity)
        .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 12))
    }
}
