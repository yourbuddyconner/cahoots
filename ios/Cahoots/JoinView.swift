import SwiftUI

/// "Join a ceremony": scan the host's code, then pick a seat.
struct JoinView: View {
    let core: Core
    @EnvironmentObject private var model: AppModel
    @State private var offer: JoinOffer?
    @State private var seat: UInt8 = 1
    @State private var error: String?
    @State private var reading = false
    @State private var joining = false

    var body: some View {
        Form {
            if let offer {
                pick(offer)
            } else {
                Section {
                    Text("Scan the host's code.").font(.headline)
                    Text("For a ceremony with codes only, scan anyone who has joined. Their code is a short animation.")
                        .foregroundStyle(.secondary)
                    ScanPanel(core: core, placeholder: "cahoots:ceremony:v1:...") { text, pasted in
                        Task { await read(text, pasted: pasted) }
                    }
                    if reading {
                        Text("Reading the animation. Hold steady.").font(.footnote).foregroundStyle(.secondary)
                    }
                    StatusLine(text: error)
                }
            }
        }
        .navigationTitle(offer?.name ?? "Join a ceremony")
        .navigationBarTitleDisplayMode(.inline)
    }

    @ViewBuilder
    private func pick(_ offer: JoinOffer) -> some View {
        Section {
            VStack(alignment: .leading, spacing: 8) {
                Text("Which seat are you?").font(.title2.weight(.semibold))
                Text("Seat 1 is the host's. Pick a free one, and make sure nobody else picks the same.")
                    .foregroundStyle(.secondary)
                Text("\(offer.members) people · \(offer.throughRelay ? "through the relay" : "codes only")")
                    .font(.footnote).foregroundStyle(.secondary)
            }
        }
        Section("Your seat") {
            Picker("Your seat", selection: $seat) {
                ForEach(1..<Int(offer.members), id: \.self) { i in
                    Text("Seat \(i + 1)").tag(UInt8(i))
                }
            }
            .pickerStyle(.inline)
            .labelsHidden()
        }
        Section {
            Button {
                Task { await join(offer) }
            } label: {
                if joining { ProgressView() } else { Text("Make my contribution") }
            }
            .disabled(joining)
            .accessibilityIdentifier("make-contribution")
            StatusLine(text: error)
        }
    }

    private func read(_ text: String, pasted: Bool) async {
        do {
            if let o = try await core.run({ try $0.joinScan(text: text) }) {
                offer = o
                seat = 1
                error = nil
                reading = false
            } else {
                reading = true
            }
        } catch {
            // The camera sees many things that are not codes. Only a pasted code is worth a word.
            if pasted { self.error = plain(error) }
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
