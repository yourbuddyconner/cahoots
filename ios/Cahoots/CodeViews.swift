import SwiftUI

/// A QR code drawn from the phone crate's modules, with a white quiet zone.
struct QRCodeView: View {
    let matrix: QrMatrix
    var caption: String?

    var body: some View {
        VStack(spacing: Space.s) {
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
            .clipShape(RoundedRectangle(cornerRadius: Radius.control))
            .overlay(RoundedRectangle(cornerRadius: Radius.control).strokeBorder(Color.line))
            .frame(maxWidth: 360)
            .frame(maxWidth: .infinity)
            .accessibilityElement()
            .accessibilityLabel("Code")
            .accessibilityIdentifier("code")
            if let caption {
                Text(caption).font(.footnote).foregroundStyle(Color.muted)
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
            ProgressView().frame(maxWidth: .infinity, minHeight: 240)
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
    @FocusState private var pasting: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: Space.l) {
            VStack(alignment: .leading, spacing: Space.s) {
                CameraScanner(core: core) { onCode($0, false) }
                    .aspectRatio(1, contentMode: .fit)
                    .frame(maxWidth: .infinity)
                    .background(Color.ink)
                    .clipShape(RoundedRectangle(cornerRadius: Radius.control))
                    .overlay(RoundedRectangle(cornerRadius: Radius.control).strokeBorder(Color.line))
                Text(hint).font(.footnote).foregroundStyle(Color.muted)
            }
            VStack(alignment: .leading, spacing: Space.s) {
                Text("No camera? Paste a code")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(Color.muted)
                HStack(alignment: .center, spacing: Space.s) {
                    TextField(placeholder, text: $pasted, axis: .vertical)
                        .lineLimit(1...4)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .font(.footnote.monospaced())
                        .focused($pasting)
                        .accessibilityIdentifier("paste-code")
                        .paperField()
                        .contentShape(Rectangle())
                        .onTapGesture { pasting = true }
                    Button("Use") {
                        let t = pasted.trimmingCharacters(in: .whitespacesAndNewlines)
                        guard !t.isEmpty else { return }
                        pasted = ""
                        onCode(t, true)
                    }
                    .buttonStyle(CompactButtonStyle())
                    .accessibilityLabel("Use this code")
                    .accessibilityIdentifier("use-code")
                }
            }
        }
    }
}

/// A line of status under a screen. A problem reads in ink beside a mark, a notice in Muted.
struct StatusLine: View {
    let text: String?
    var problem = true

    var body: some View {
        if let text, !text.isEmpty {
            HStack(alignment: .firstTextBaseline, spacing: Space.s) {
                if problem {
                    Image(systemName: "exclamationmark.circle")
                        .foregroundStyle(Color.ink)
                        .accessibilityHidden(true)
                }
                Text(text)
                    .font(.subheadline)
                    .foregroundStyle(problem ? Color.ink : Color.muted)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("status")
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// Shown while the seat key waits for Face ID.
struct FaceIDBanner: View {
    var body: some View {
        HStack(spacing: Space.m) {
            Image(systemName: "faceid").foregroundStyle(Color.brand).accessibilityHidden(true)
            Text("Confirm with Face ID").foregroundStyle(Color.ink)
            Spacer()
            ProgressView()
        }
        .padding(.horizontal, Space.l)
        .frame(minHeight: 52)
        .overlay(RoundedRectangle(cornerRadius: Radius.control).strokeBorder(Color.line))
        .accessibilityElement(children: .combine)
    }
}
