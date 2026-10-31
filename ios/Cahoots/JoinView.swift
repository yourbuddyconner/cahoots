import SwiftUI

/// "Join a circle": scan the host's code, then pick a seat.
struct JoinView: View {
    let core: Core
    @EnvironmentObject private var model: AppModel
    @State private var offer: JoinOffer?
    @State private var seat: UInt8 = 1
    @State private var error: String?
    @State private var reading = false
    @State private var joining = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xl) {
                if let offer {
                    pick(offer)
                } else {
                    Text("Scan the code on the phone of the person creating the circle. If you’re using Codes only, you can scan anyone’s step 1 code.")
                        .font(.body)
                        .foregroundStyle(Color.muted)
                        .fixedSize(horizontal: false, vertical: true)
                    ScanPanel(core: core, placeholder: "cahoots:ceremony:v1:...") { text, pasted in
                        Task { await read(text, pasted: pasted) }
                    }
                    if reading {
                        StatusLine(text: "Reading the animation. Hold steady.", problem: false)
                    }
                    StatusLine(text: error)
                }
            }
            .padding(.horizontal, Space.l)
            .padding(.top, Space.s)
            .padding(.bottom, Space.xl)
        }
        .scrollDismissesKeyboard(.interactively)
        .paperBackground()
        .navigationTitle(offer?.name ?? "Join a circle")
        .navigationBarTitleDisplayMode(offer == nil ? .large : .inline)
        .actionBar(shown: offer != nil) {
            if let offer {
                StatusLine(text: error)
                Button {
                    Task { await join(offer) }
                } label: {
                    Working(busy: joining, title: "Join this circle")
                }
                .buttonStyle(.filled)
                .disabled(joining)
                .accessibilityIdentifier("make-contribution")
            }
        }
    }

    @ViewBuilder
    private func pick(_ offer: JoinOffer) -> some View {
        ScreenHeader(title: "Which seat are you?",
                     detail: "A seat is your device’s place in the circle. The host uses seat 1. Choose another number and check that no one else is using it.",
                     style: .title)
        Text("\(spelled(Int(offer.members))) people, \(offer.throughRelay ? "through the relay" : "with codes only").")
            .font(.subheadline)
            .foregroundStyle(Color.muted)
        PaperSection(title: "Your seat") {
            ForEach(1..<Int(offer.members), id: \.self) { i in
                if i > 1 { Hairline() }
                if i <= Int(offer.badges) {
                    cardRow(i + 1)
                } else {
                    OptionRow(title: "Seat \(i + 1)", chosen: seat == UInt8(i)) { seat = UInt8(i) }
                }
            }
        }
        .sensoryFeedback(.selection, trigger: seat)
    }

    /// A seat a card in the pile takes. It cannot be chosen.
    private func cardRow(_ number: Int) -> some View {
        HStack {
            Text("Seat \(number), a card").font(.body).foregroundStyle(Color.muted)
            Spacer()
        }
        .padding(.vertical, Space.m)
        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isStaticText)
    }

    private func read(_ text: String, pasted: Bool) async {
        do {
            if let o = try await core.run({ try $0.joinScan(text: text) }) {
                withAnimation(.snappy) {
                    offer = o
                    seat = o.badges + 1
                    error = nil
                    reading = false
                }
            } else {
                reading = true
            }
        } catch {
            // The camera sees many things that are not codes. Only a pasted code, or a host's
            // code the app cannot read (one from a newer version), is worth a word.
            let hostCode = text.trimmingCharacters(in: .whitespacesAndNewlines).hasPrefix("cahoots:ceremony:")
            if pasted || hostCode { self.error = plain(error) }
        }
    }

    private func join(_ offer: JoinOffer) async {
        joining = true
        defer { joining = false }
        do {
            let s = seat
            let id = try await core.run {
                offer.throughRelay
                    ? try $0.joinOffer(text: offer.paramsCode, seat: s)
                    : try $0.joinCeremony(paramsCode: offer.paramsCode, seat: s)
            }
            model.joined(id)
        } catch {
            self.error = plain(error)
        }
    }
}
