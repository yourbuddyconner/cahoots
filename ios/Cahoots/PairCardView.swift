import SwiftUI

/// The two taps of pairing with a badge, as the pairing screen asks for them. `Core` does them
/// on the phone's queue; the tests stand in for it.
protocol CardPairing: Sendable {
    /// The first tap: the badge's name, its circles, and the four words.
    func startPairing(name: String) async throws -> PairPreview
    /// The second tap, after both buttons on the badge: the names of the circles kept.
    func finishPairing() async throws -> [String]
}

extension Core: CardPairing {
    func startPairing(name: String) async throws -> PairPreview {
        try await run { try $0.pairCardStart(name: name) }
    }

    func finishPairing() async throws -> [String] {
        try await run { phone in
            let kept = try phone.pairCardFinish()
            let circles = phone.circles()
            return kept.compactMap { id in circles.first { $0.id == id }?.name }
        }
    }
}

/// Where the pairing is: the name and the first tap, the words and the second tap, or done.
@MainActor
final class PairCardModel: ObservableObject {
    enum Step: Equatable {
        case name
        case check(PairPreview)
        case done(badge: String, circles: [String])
    }

    let pairing: CardPairing
    @Published var name = ""
    @Published private(set) var step: Step = .name
    @Published private(set) var working = false
    @Published private(set) var error: String?

    init(pairing: CardPairing) {
        self.pairing = pairing
    }

    /// The first tap. On a problem the screen stays on the name, with the problem under it.
    func start() async {
        guard !working else { return }
        working = true
        defer { working = false }
        error = nil
        do {
            step = .check(try await pairing.startPairing(name: name))
        } catch {
            self.error = plain(error)
        }
    }

    /// The second tap. On a problem the words stay, so the person can press both and tap again.
    func finish() async {
        guard !working, case let .check(preview) = step else { return }
        working = true
        defer { working = false }
        error = nil
        do {
            let kept = try await pairing.finishPairing()
            step = .done(badge: preview.badgeName, circles: kept)
        } catch {
            self.error = plain(error)
        }
    }
}

/// Pair this phone with a badge: one tap, the words checked on the badge and both buttons
/// pressed there, then a second tap. Every circle on the badge comes with it.
struct PairCardView: View {
    @EnvironmentObject private var app: AppModel
    @StateObject private var model: PairCardModel

    init(core: Core) {
        _model = StateObject(wrappedValue: PairCardModel(pairing: core))
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xl) {
                switch model.step {
                case .name: PairNameStep(name: $model.name)
                case let .check(preview): PairCheckStep(preview: preview)
                case let .done(badge, circles): PairDoneStep(badge: badge, circles: circles)
                }
                StatusLine(text: model.error)
            }
            .padding(.horizontal, Space.l)
            .padding(.top, Space.s)
            .padding(.bottom, Space.xl)
        }
        .scrollDismissesKeyboard(.interactively)
        .paperBackground()
        .navigationTitle("Pair a card")
        .navigationBarTitleDisplayMode(.inline)
        .navigationBarBackButtonHidden(model.working)
        .actionBar { action }
    }

    @ViewBuilder
    private var action: some View {
        switch model.step {
        case .name:
            Button { Task { await model.start() } } label: {
                Working(busy: model.working, title: "Hold the phone to the card")
            }
            .buttonStyle(.filled)
            .disabled(model.working)
            .accessibilityLabel("Hold the phone to the card")
            .accessibilityIdentifier("pair-start")
        case .check:
            Button { Task { await model.finish() } } label: {
                Working(busy: model.working, title: "Hold the phone again")
            }
            .buttonStyle(.filled)
            .disabled(model.working)
            .accessibilityLabel("Hold the phone again")
            .accessibilityIdentifier("pair-finish")
        case .done:
            Button("Done") { app.leave() }
                .buttonStyle(.filled)
                .accessibilityIdentifier("pair-done")
        }
    }
}

/// The first step: what pairing allows, and the name the badge will show for this phone.
private struct PairNameStep: View {
    @Binding var name: String

    var body: some View {
        ScreenHeader(title: "Pair a card",
                     detail: "This phone can read and post in this badge's circles until you unpair it on the badge.",
                     style: .title)
        VStack(alignment: .leading, spacing: Space.s) {
            Text("Name for this phone")
                .font(.footnote.weight(.semibold)).foregroundStyle(Color.muted)
                .accessibilityAddTraits(.isHeader)
            TextField(UIDevice.current.name, text: $name)
                .textInputAutocapitalization(.words)
                .autocorrectionDisabled()
                .submitLabel(.done)
                .paperField()
                .accessibilityLabel("Name for this phone")
                .accessibilityIdentifier("pair-name")
        }
    }
}

/// The second step: the four words to check against the badge's screen.
private struct PairCheckStep: View {
    let preview: PairPreview

    var body: some View {
        ScreenHeader(title: preview.badgeName,
                     detail: "Open 'pair a phone' on your badge and check the words match. Press both buttons, then hold the phone to the card again.",
                     style: .title)
        VStack(alignment: .leading, spacing: Space.xs) {
            ForEach(Array(preview.words.enumerated()), id: \.offset) { _, word in
                Text(word).serifTitle(.largeTitle)
            }
        }
        .padding(.vertical, Space.s)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("pair-words")
        CircleNames(title: "Circles on this badge", names: preview.circles)
    }
}

/// The last step: the circles now on this phone.
private struct PairDoneStep: View {
    let badge: String
    let circles: [String]

    var body: some View {
        ScreenHeader(title: "Paired with \(badge)", style: .title)
        if circles.isEmpty {
            StatusLine(text: "No circles came from this badge.")
        } else {
            CircleNames(title: "Circles on this phone", names: circles)
        }
    }
}

private struct CircleNames: View {
    let title: String
    let names: [String]

    var body: some View {
        PaperSection(title: title) {
            ForEach(Array(names.enumerated()), id: \.offset) { i, name in
                if i > 0 { Hairline() }
                Text(name.isEmpty ? "A circle with no name" : name)
                    .serifTitle(.title3)
                    .padding(.vertical, Space.m)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }
}
