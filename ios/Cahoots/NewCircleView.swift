import SwiftUI

/// "Create a circle": a name, how many people, how to hold the ceremony, and the secret's size.
struct NewCircleView: View {
    @EnvironmentObject private var model: AppModel
    @State private var name = ""
    @State private var members = 2
    @State private var badges = 0
    @State private var throughRelay = true
    @State private var size = Self.sizes[1].bytes
    @State private var error: String?
    @State private var starting = false
    @State private var advanced = false
    @FocusState private var naming: Bool

    static let sizes: [(bytes: UInt32, label: String)] = [
        (16384, "Small"),
        (65536, "Medium"),
        (262144, "Large"),
    ]

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xxl) {
                Text("Start with everyone in the same room. Name your circle, then follow the code exchange on each phone. This setup is called a ceremony.")
                    .font(.body)
                    .foregroundStyle(Color.muted)
                    .fixedSize(horizontal: false, vertical: true)

                VStack(alignment: .leading, spacing: Space.s) {
                    Text("Name")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(Color.muted)
                    TextField("park, book club, family", text: $name)
                        .textInputAutocapitalization(.words)
                        .autocorrectionDisabled()
                        .submitLabel(.done)
                        .focused($naming)
                        .onSubmit { naming = false }
                        .paperField()
                        .accessibilityLabel("Name")
                        .accessibilityIdentifier("circle-name")
                }

                PaperSection(title: "People in the room",
                             footer: "Count everyone joining on a separate device, including you. Each device gets a numbered seat.") {
                    PeopleStepper(value: $members, range: 2...8)
                }
                .onChange(of: members) { _, m in badges = min(badges, m - 1) }

                if throughRelay {
                    PaperSection(title: "Cards in the pile",
                                 footer: "Badges stacked on a laptop running the bridge. They take the seats right after yours.") {
                        PeopleStepper(value: $badges, range: 0...(members - 1), noun: "card", label: "Cards in the pile")
                    }
                }

                PaperSection(title: "How to connect") {
                    OptionRow(title: "Through the relay",
                              detail: "Use an internet connection. Everyone scans your code, then you scan theirs. The relay handles the remaining steps.",
                              chosen: throughRelay) { throughRelay = true }
                    Hairline()
                    OptionRow(title: "Codes only",
                              detail: "Everyone shows and scans codes. No network is needed.",
                              chosen: !throughRelay) { throughRelay = false; badges = 0 }
                }

                DisclosureGroup("Advanced options", isExpanded: $advanced) {
                    PaperSection(title: "Shared secret size", footer: "Larger secrets take longer to exchange with codes only.") {
                        ForEach(Array(Self.sizes.enumerated()), id: \.offset) { i, s in
                            if i > 0 { Hairline() }
                            OptionRow(title: s.label, chosen: size == s.bytes) { size = s.bytes }
                        }
                    }
                    .padding(.top, Space.m)
                }
                .foregroundStyle(Color.ink)
            }
            .padding(.horizontal, Space.l)
            .padding(.top, Space.s)
            .padding(.bottom, Space.xl)
        }
        .scrollDismissesKeyboard(.interactively)
        .paperBackground()
        .navigationTitle("Create a circle")
        .navigationBarTitleDisplayMode(.large)
        .sensoryFeedback(.selection, trigger: throughRelay)
        .sensoryFeedback(.selection, trigger: size)
        .actionBar {
            StatusLine(text: error)
            Button {
                Task { await start() }
            } label: {
                Working(busy: starting, title: "Start the ceremony")
            }
            .buttonStyle(.filled)
            .disabled(starting || name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            .accessibilityIdentifier("start-ceremony")
        }
    }

    private func start() async {
        let n = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !n.isEmpty else {
            error = "Enter a name for your circle."
            naming = true
            return
        }
        starting = true
        defer { starting = false }
        do {
            try await model.startCeremony(name: n, members: UInt8(members), badges: UInt8(throughRelay ? badges : 0), padLen: size, throughRelay: throughRelay)
        } catch {
            self.error = plain(error)
        }
    }
}

/// How many people: the count in words, and two large targets to change it.
struct PeopleStepper: View {
    @Binding var value: Int
    let range: ClosedRange<Int>
    var noun = "person"
    var label = "People in the room"

    var body: some View {
        HStack(spacing: Space.m) {
            Text("\(value) \(value == 1 ? noun : noun == "person" ? "people" : noun + "s")")
                .font(.body.monospacedDigit())
                .foregroundStyle(Color.ink)
                .contentTransition(.numericText(value: Double(value)))
            Spacer()
            step("minus", by: -1).disabled(value <= range.lowerBound)
            step("plus", by: 1).disabled(value >= range.upperBound)
        }
        .padding(.vertical, Space.s)
        .sensoryFeedback(.selection, trigger: value)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label)
        .accessibilityValue("\(value)")
        .accessibilityAdjustableAction { direction in
            switch direction {
            case .increment: if value < range.upperBound { value += 1 }
            case .decrement: if value > range.lowerBound { value -= 1 }
            @unknown default: break
            }
        }
        .accessibilityIdentifier(noun == "person" ? "people" : "cards")
    }

    private func step(_ symbol: String, by delta: Int) -> some View {
        Button {
            withAnimation(.snappy) { value += delta }
        } label: {
            Image(systemName: symbol)
                .font(.body.weight(.semibold))
                .frame(width: 44, height: 44)
                .overlay(Circle().strokeBorder(Color.ink.opacity(0.22)))
                .contentShape(Circle())
        }
        .buttonStyle(StepperButtonStyle())
    }
}

private struct StepperButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(Color.ink)
            .opacity(enabled ? (configuration.isPressed ? 0.5 : 1) : 0.3)
    }
}
