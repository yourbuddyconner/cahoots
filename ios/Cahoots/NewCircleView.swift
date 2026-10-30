import SwiftUI

/// "Create a circle": a name, how many people, how to hold the ceremony, and the secret's size.
struct NewCircleView: View {
    @EnvironmentObject private var model: AppModel
    @State private var name = ""
    @State private var members = 2
    @State private var throughRelay = true
    @State private var size = Self.sizes[1].bytes
    @State private var error: String?
    @State private var starting = false

    static let sizes: [(bytes: UInt32, label: String, hint: String)] = [
        (16384, "Small", "16 KiB · about 100 messages · a few seconds to scan"),
        (65536, "Medium", "64 KiB · a few hundred messages · about half a minute to scan"),
        (262144, "Large", "256 KiB · a couple thousand messages · about two minutes to scan"),
    ]

    var body: some View {
        Form {
            Section {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Bring your people together.").font(.title2.weight(.semibold))
                    Text("Give your circle a name, then exchange codes with the people beside you.")
                        .foregroundStyle(.secondary)
                }
            }
            Section("Name") {
                TextField("e.g. sf-meetup", text: $name)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .accessibilityIdentifier("circle-name")
            }
            Section {
                Stepper("\(members) people", value: $members, in: 2...8)
                    .accessibilityIdentifier("people")
            } header: {
                Text("People in the room")
            } footer: {
                Text("Count every device that will hold the secret, including yours.")
            }
            Section {
                Picker("How to hold the ceremony", selection: $throughRelay) {
                    Text("Through the relay").tag(true)
                    Text("Codes only").tag(false)
                }
                .pickerStyle(.segmented)
            } header: {
                Text("How to hold the ceremony")
            } footer: {
                Text(throughRelay
                     ? "Everyone scans one code from your screen. The rest goes through the relay."
                     : "Everyone shows and scans codes. No network is needed.")
            }
            Section {
                Picker("Secret size", selection: $size) {
                    ForEach(Self.sizes, id: \.bytes) { Text($0.label).tag($0.bytes) }
                }
                .pickerStyle(.segmented)
            } header: {
                Text("Secret size")
            } footer: {
                Text(Self.sizes.first { $0.bytes == size }?.hint ?? "")
            }
            Section {
                Button {
                    Task { await start() }
                } label: {
                    if starting { ProgressView() } else { Text("Continue") }
                }
                .disabled(starting)
                StatusLine(text: error)
            }
        }
        .navigationTitle("Create a circle")
        .navigationBarTitleDisplayMode(.inline)
    }

    private func start() async {
        let n = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !n.isEmpty else {
            error = "Give the circle a name."
            return
        }
        starting = true
        defer { starting = false }
        do {
            try await model.startCeremony(name: n, members: UInt8(members), padLen: size, throughRelay: throughRelay)
        } catch {
            self.error = plain(error)
        }
    }
}
