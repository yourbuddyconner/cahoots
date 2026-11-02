import SwiftUI

/// The badge this phone is paired with: its name and face, when it was last tapped, what a tap
/// would do now, and one button that taps. Forgetting the card is here too. With no card
/// paired it offers to pair one.
struct CardView: View {
    @EnvironmentObject private var app: AppModel
    @State private var tapping = false
    /// What the last tap did, in one sentence.
    @State private var result: String?
    @State private var error: String?
    @State private var confirmForget = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xl) {
                if let status = app.cardStatus {
                    if status.paired {
                        paired(status)
                    } else {
                        ScreenHeader(title: "No card on this phone",
                                     detail: "Pair this phone with a badge to read and post in the badge’s circles.",
                                     style: .title)
                    }
                } else {
                    ProgressView().frame(maxWidth: .infinity)
                }
                StatusLine(text: result, problem: false)
                StatusLine(text: error)
                if app.cardStatus?.paired == true {
                    Button("Forget this card") { confirmForget = true }
                        .buttonStyle(TextButtonStyle(full: false, color: .ink))
                        .disabled(tapping)
                        .accessibilityIdentifier("forget-card")
                }
            }
            .padding(.horizontal, Space.l)
            .padding(.top, Space.s)
            .padding(.bottom, Space.xl)
        }
        .paperBackground()
        .navigationTitle("Card")
        .navigationBarTitleDisplayMode(.inline)
        .navigationBarBackButtonHidden(tapping)
        .actionBar(shown: app.cardStatus != nil) {
            if app.cardStatus?.paired == true {
                Button { Task { await tap() } } label: {
                    Working(busy: tapping, title: "Tap the card")
                }
                .buttonStyle(.filled)
                .disabled(tapping)
                .accessibilityLabel("Tap the card")
                .accessibilityIdentifier("tap-card")
            } else {
                NavigationLink(value: Route.pairCard) { Text("Pair a card") }
                    .buttonStyle(.filled)
                    .accessibilityIdentifier("card-pair-card")
            }
        }
        .confirmationDialog("Forget this card?", isPresented: $confirmForget, titleVisibility: .visible) {
            Button("Forget this card", role: .destructive) {
                Task {
                    do { try await app.forgetCard() } catch { self.error = plain(error) }
                }
            }
        } message: {
            Text("This deletes the card’s circles, their messages and the card’s key from this phone. On the badge, open “phone” in the menu and unpair this phone before pairing again.")
        }
        .task { await app.refreshCard() }
    }

    /// The badge's name and face, the last tap, and what a tap would do now.
    private func paired(_ status: CardStatus) -> some View {
        let name = Self.badgeName(status, cardName: badgeCardName)
        return VStack(alignment: .leading, spacing: Space.xl) {
            HStack(spacing: Space.m) {
                if let face = badgeFace {
                    FaceView(pixels: face, size: 64, label: "\(name)'s face")
                }
                ScreenHeader(title: name, detail: Self.lastTap(status.lastTap), style: .title)
            }
            PaperSection(title: "What happens when you tap the card") {
                ForEach(Array(Self.summary(status) { app.seatName(circle: $0, seat: $1) }.enumerated()), id: \.offset) { i, line in
                    if i > 0 { Hairline() }
                    Text(line)
                        .font(.body).foregroundStyle(Color.ink)
                        .padding(.vertical, Space.m)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .accessibilityIdentifier("card-summary")
        }
    }

    /// The name on the card the badge published in a card circle, as the Circles list shows
    /// it. Nil while it has published none.
    private var badgeCardName: String? {
        guard let c = app.circles.first(where: AppModel.isCard),
              app.hasCardName(circle: c.id, seat: c.mySeat) else { return nil }
        return app.seatName(circle: c.id, seat: c.mySeat)
    }

    /// The badge's name on this screen: the name on its card, else the crate's, which is
    /// "your badge" since the badge writes no name of its own.
    static func badgeName(_ status: CardStatus, cardName: String?) -> String {
        cardName ?? status.badge
    }

    /// The badge's face: a card circle's own seat is the badge's, from the card it published.
    private var badgeFace: Data? {
        guard let c = app.circles.first(where: AppModel.isCard),
              let faces = app.faces[c.id], Int(c.mySeat) < faces.count else { return nil }
        return faces[Int(c.mySeat)]
    }

    private func tap() async {
        guard !tapping else { return }
        tapping = true
        defer { tapping = false }
        result = nil
        error = nil
        do {
            result = Self.result(try await app.tapCard())
        } catch {
            self.error = Self.problem(error)
        }
    }

    /// What a tap would do now, a line each: "up to date" when nothing waits. `name` gives a
    /// seat's name in a circle.
    static func summary(_ s: CardStatus, name: (String, UInt8) -> String) -> [String] {
        var lines: [String] = []
        if s.keysWanted > 0 {
            lines.append(s.keysWanted == 1 ? "1 circle needs a key" : "\(s.keysWanted) circles need a key")
        }
        if s.bumpsToPost > 0 {
            lines.append(counted(s.bumpsToPost, "bump") + " to post")
        }
        for low in s.wordsLow {
            lines.append("hello words: \(low.left) left for \(name(low.circle, low.seat))")
        }
        return lines.isEmpty ? ["up to date"] : lines
    }

    /// What one tap did, in one sentence: "Fetched 2 keys and 16 hello words, passed on
    /// 1 bump, and kept 1 new circle." A bump passed on reached the relay, or was there
    /// already. A new circle is one the badge kept after this phone paired.
    static func result(_ r: TapReport) -> String {
        var fetched: [String] = []
        if r.keys > 0 { fetched.append(counted(r.keys, "key")) }
        if r.words > 0 { fetched.append(counted(r.words, "hello word")) }
        var parts: [String] = []
        if !fetched.isEmpty { parts.append("fetched \(fetched.joined(separator: " and "))") }
        if r.bumps > 0 { parts.append("passed on \(counted(r.bumps, "bump"))") }
        if r.kept > 0 { parts.append("kept \(counted(r.kept, "new circle"))") }
        guard let last = parts.popLast() else { return "Nothing was waiting." }
        let sentence = parts.isEmpty ? last : parts.joined(separator: ", ") + ", and " + last
        return sentence.prefix(1).uppercased() + sentence.dropFirst() + "."
    }

    /// "last tap: never", or how long ago, as "last tap: 2 minutes ago".
    static func lastTap(_ seconds: UInt64?, now: Date = Date()) -> String {
        guard let seconds else { return "last tap: never" }
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .full
        f.dateTimeStyle = .named
        let when = min(Date(timeIntervalSince1970: TimeInterval(seconds)), now)
        return "last tap: " + f.localizedString(for: when, relativeTo: now)
    }

    /// A tap's problem as this screen says it, with what to do next. A badge closed to taps
    /// gives the phone no chip, or one that goes quiet in the middle of a tap, so those two
    /// say how to open the window again. A card that no longer knows this phone points at
    /// Forget.
    static func problem(_ error: Error) -> String {
        let text = plain(error)
        switch error {
        case PhoneError.NoCard:
            return text + " " + closedHint
        case let PhoneError.Nfc(m) where m == NfcTransport.movedAway:
            return text + " " + closedHint
        case PhoneError.NotPaired:
            return text + " Choose Forget this card, then pair again."
        default:
            return text
        }
    }

    /// The badge's left button opens its window for a tap on its home page. While the window
    /// is open the same button closes it, so the hint names the case it is for.
    static let closedHint = "If the badge says closed to taps, press its left button, then try again."

    /// "1 bump", "2 bumps".
    private static func counted(_ n: UInt32, _ thing: String) -> String {
        "\(n) \(thing)\(n == 1 ? "" : "s")"
    }
}
