import SwiftUI

/// A guide shown once at first launch and available later from Settings.
struct WelcomeView: View {
    let core: Core
    var firstVisit = true
    let done: () -> Void
    @State private var page = 0
    @State private var faces: [Data] = []
    @AccessibilityFocusState private var headingFocused: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private let pages: [WelcomePage] = [
        WelcomePage(title: "A little circle of your people",
                    detail: "Meet in person to create a private circle. Then keep talking, wherever you go. No account or phone number needed.",
                    first: ("Set up together", "One person creates the circle. Everyone exchanges codes and checks the same six words. We call this setup a ceremony."),
                    second: ("Give everyone a name", "Each device gets a numbered seat. In People, add familiar names that only you see.")),
        WelcomePage(title: "A familiar voice, a little more certainty",
                    detail: "Hello words help you check someone on a call using a secret shared by your two devices.",
                    first: ("Open each other’s seat", "In the same circle, open People and select the person you’re talking to. Make sure both phones show the same hello number."),
                    second: ("Take turns saying two words", "Read your own pair aloud and check their reply. Don’t tell them the words you expect to hear. When both pairs match, each of you taps Next hello.")),
        WelcomePage(title: "Your circle can reach a little further",
                    detail: "A channel lets your circle share messages with a wider audience. Choose who can read and who can post.",
                    first: ("Share a link thoughtfully", "Anyone with a channel link can read and forward it. Choose Closed for a private channel without a shared link."),
                    second: ("You control posting", "A link doesn’t automatically let someone post. Review requests in the channel’s settings. You can remove a member’s future access later.")),
        WelcomePage(title: "Yours to keep. Here on this phone.",
                    detail: "Cahoots stores your keys and history on this phone. There is no account to sign back into or restore them from.",
                    first: ("Keep this phone", "Circles are not included in backups and cannot transfer to a new phone. To use another phone, create a new circle together. Removing a circle deletes its local messages and keys."),
                    second: ("Know what leaves your phone", "A relay delivers encrypted messages without being able to read them. Exported history is readable, so share those files carefully.")),
    ]

    var body: some View {
        NavigationStack {
            pageContent
                .paperBackground()
                .navigationTitle(firstVisit ? "Welcome" : "Cahoots guide")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar { navigationActions }
                .actionBar { pageActions }
                .task { await loadFaces() }
        }
        .interactiveDismissDisabled(firstVisit)
    }

    private var pageContent: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xl) {
                WelcomeArt(page: page, faces: faces)
                VStack(alignment: .leading, spacing: Space.xl) {
                    ScreenHeader(title: pages[page].title, detail: pages[page].detail, style: .largeTitle)
                        .accessibilityElement(children: .combine)
                        .accessibilityIdentifier("welcome-page-\(page)")
                        .accessibilityFocused($headingFocused)
                    InstructionStep(number: 1, title: pages[page].first.0, detail: pages[page].first.1)
                    InstructionStep(number: 2, title: pages[page].second.0, detail: pages[page].second.1)
                }
                .id(page)
                .transition(reduceMotion ? .identity : .opacity.combined(with: .offset(y: 8)))
                Text("You can revisit this guide in Settings.")
                    .font(.footnote).foregroundStyle(Color.muted)
            }
            .padding(Space.l)
        }
        .id(page)
        .task(id: page) {
            await Task.yield()
            headingFocused = true
        }
    }

    @ToolbarContentBuilder
    private var navigationActions: some ToolbarContent {
        ToolbarItem(placement: .cancellationAction) {
            Button(firstVisit ? "Skip for now" : "Close", action: done)
                .accessibilityIdentifier("welcome-close")
        }
        if page > 0 {
            ToolbarItem(placement: .topBarTrailing) {
                Button("Back") { move(to: page - 1) }
                    .accessibilityIdentifier("welcome-back")
            }
        }
    }

    private var pageActions: some View {
        VStack(spacing: Space.s) {
            HStack(spacing: Space.s) {
                ForEach(0..<pages.count, id: \.self) { index in
                    Capsule().fill(index == page ? Color.brand : Color.line)
                        .frame(width: index == page ? 24 : 8, height: 6)
                }
                Spacer()
                Text("\(page + 1) of \(pages.count)").font(.footnote.monospacedDigit()).foregroundStyle(Color.muted)
            }
            .padding(.bottom, Space.s)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Page \(page + 1) of \(pages.count)")
            Button(page == pages.count - 1 ? "Get started" : "Continue") {
                if page == pages.count - 1 { done() } else { move(to: page + 1) }
            }
            .buttonStyle(.filled)
            .accessibilityIdentifier("welcome-next")
        }
    }

    private func loadFaces() async {
        faces = (try? await core.run { phone -> [Data] in
            var result: [Data] = []
            for index in 0..<3 {
                let bytes: [UInt8] = (0..<32).map { byte in UInt8((byte * 17 + index * 47) % 256) }
                result.append(phone.facePixels(seed: Data(bytes)))
            }
            return result
        }) ?? []
    }

    private func move(to next: Int) {
        headingFocused = false
        withAnimation(reduceMotion ? nil : .easeInOut(duration: 0.24)) { page = next }
    }
}

private struct WelcomePage {
    let title: String
    let detail: String
    let first: (String, String)
    let second: (String, String)
}

/// Small, illustrative cahoots gather, exchange words, share, and settle beside a lock.
/// These demonstration faces are decorative, not a preview of real members or their identities.
private struct WelcomeArt: View {
    let page: Int
    let faces: [Data]
    @State private var floating = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var symbol: String {
        ["heart.fill", "arrow.left.arrow.right", "paperplane.fill", "lock.fill"][page]
    }

    var body: some View {
        GeometryReader { geometry in
            let center = geometry.size.width / 2
            ZStack {
                Circle().stroke(Color.brand.opacity(0.15), style: StrokeStyle(lineWidth: 1.5, dash: [4, 7]))
                    .frame(width: page == 2 ? 190 : 150, height: page == 2 ? 190 : 150)
                    .position(x: center, y: 90)
                ForEach(0..<3, id: \.self) { index in
                    FaceView(pixels: faces.indices.contains(index) ? faces[index] : nil, size: index == 1 ? 64 : 52)
                        .rotationEffect(.degrees(reduceMotion ? 0 : (index == 0 ? -6 : index == 2 ? 6 : 0) + (floating ? 2 : -2)))
                        .offset(y: reduceMotion ? 0 : (floating ? -3 : 3))
                        .animation(reduceMotion ? nil : .easeInOut(duration: 1.1).repeatCount(2, autoreverses: true), value: floating)
                        .position(x: center + CGFloat(index - 1) * (page == 2 ? 86 : 70),
                                  y: index == 1 ? 64 : 102)
                        .transaction { $0.animation = nil }
                }
                Image(systemName: symbol)
                    .font(.title2.weight(.medium)).foregroundStyle(Color.brand)
                    .contentTransition(reduceMotion ? .identity : .symbolEffect(.replace))
                    .position(x: center, y: 145)
            }
        }
        .frame(height: 180)
        .animation(reduceMotion ? nil : .spring(duration: 0.6, bounce: 0.18), value: page)
        .task {
            // Let the first layout settle before animating only the decorative transforms.
            try? await Task.sleep(for: .milliseconds(250))
            guard !Task.isCancelled else { return }
            floating = !reduceMotion
        }
        .onChange(of: reduceMotion) { _, reduced in floating = !reduced }
        .accessibilityHidden(true)
    }
}
