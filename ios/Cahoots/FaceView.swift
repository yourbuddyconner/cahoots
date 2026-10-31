import SwiftUI

/// A cahoot: the 24 x 24 portrait drawn in ink on cream, with a one-pixel cream margin and square
/// corners, as on the badge's e-paper screen.
struct FaceView: View {
    /// 576 bytes, 24 x 24 row-major, 1 for ink: `Phone.facePixels`.
    let pixels: Data?
    var size: CGFloat = 44
    /// What VoiceOver says. Until traits give the face words, which seat it is.
    var label: String?

    var body: some View {
        Canvas { ctx, canvas in
            ctx.fill(Path(CGRect(origin: .zero, size: canvas)), with: .color(.cream))
            guard let pixels, pixels.count == 576 else { return }
            let cell = canvas.width / 26
            var ink = Path()
            let px = [UInt8](pixels)
            for y in 0..<24 {
                for x in 0..<24 where px[y * 24 + x] == 1 {
                    ink.addRect(CGRect(x: CGFloat(x + 1) * cell, y: CGFloat(y + 1) * cell,
                                       width: cell, height: cell))
                }
            }
            ctx.fill(ink, with: .color(Color(red: 0.09, green: 0.094, blue: 0.106)))
        }
        .frame(width: size, height: size)
        .accessibilityElement()
        .accessibilityLabel(label ?? "")
        .accessibilityHidden(label == nil)
    }
}

/// A few faces side by side, a little apart, for a circle's members.
struct FaceRow: View {
    let faces: [Data]
    var size: CGFloat = 20
    var limit = 5

    var body: some View {
        HStack(spacing: 3) {
            ForEach(Array(faces.prefix(limit).enumerated()), id: \.offset) { _, face in
                FaceView(pixels: face, size: size)
            }
            if faces.count > limit {
                Text("+\(faces.count - limit)")
                    .font(.caption.weight(.medium))
                    .foregroundStyle(Color.muted)
            }
        }
        .accessibilityHidden(true)
    }
}

extension Phone {
    /// A seat's face in a circle, drawn: 576 bytes of pixels, or nil.
    func seatFace(circle: String, seat: UInt8) -> Data? {
        (try? seatFaceSeed(circle: circle, seat: seat)).map { facePixels(seed: $0) }
    }

    /// Every seat's face in a circle, by seat.
    func faces(circle: String, members: UInt8) -> [UInt8: Data] {
        Dictionary(uniqueKeysWithValues: (0..<members).compactMap { seat in
            seatFace(circle: circle, seat: seat).map { (seat, $0) }
        })
    }

    /// This phone's own face, drawn.
    func myFace() -> Data { facePixels(seed: myFaceSeed()) }
}
