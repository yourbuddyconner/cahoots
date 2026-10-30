import CryptoKit
import SwiftUI

/// This phone's key, where its secrets live, the relay for new circles, and each circle's
/// export and removal.
struct SettingsView: View {
    let core: Core
    @EnvironmentObject private var app: AppModel
    @State private var info: PhoneInfo?
    @State private var relay = ""
    @State private var relayNote: String?
    @State private var relayError: String?
    @State private var export: ExportText?
    @State private var removing: CircleSummary?
    @State private var error: String?

    var body: some View {
        List {
            Section("This phone") {
                if let info {
                    Text(info.keyStore.prefix(1).uppercased() + info.keyStore.dropFirst() + ".")
                    if !core.signer.usesEnclave {
                        Text("This simulator has no Secure Enclave. The key is in the keychain.")
                            .foregroundStyle(.orange)
                    }
                    LabeledContent("This phone's key") {
                        Text(info.fingerprint).monospaced()
                    }
                    LabeledContent("Version", value: info.appVersion)
                }
                Text("This phone's seats do not move. Seat keys never leave this phone, and backups leave out every circle. A new phone means new circles.")
                    .font(.footnote).foregroundStyle(.secondary)
            }
            Section {
                TextField("https://relay.example.com", text: $relay)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .accessibilityIdentifier("default-relay")
                Button("Save") { Task { await saveRelay() } }
                    .accessibilityIdentifier("save-relay")
                StatusLine(text: relayNote, problem: false)
                StatusLine(text: relayError)
            } header: {
                Text("Relay for new circles")
            } footer: {
                Text("A ceremony held through the relay uses the host's relay, and the circle keeps it. Change a circle's relay from its chat.")
            }
            Section {
                Text("While Cahoots is closed, iOS decides when it may check for new messages. The number on the icon is how many are unread.")
                    .font(.footnote).foregroundStyle(.secondary)
            } header: {
                Text("Checking for messages")
            }
            Section {
                ForEach(app.circles, id: \.id) { c in
                    VStack(alignment: .leading, spacing: 8) {
                        Text(c.name).font(.headline)
                        Text(c.relay).font(.footnote).foregroundStyle(.secondary)
                        HStack {
                            Button("Export history") { Task { await exportHistory(c) } }
                                .buttonStyle(.bordered)
                            Spacer()
                            Button("Remove", role: .destructive) { removing = c }
                                .buttonStyle(.bordered)
                        }
                    }
                    .padding(.vertical, 4)
                }
                StatusLine(text: error)
            } header: {
                Text("Circles")
            } footer: {
                Text("Export is a readable copy of the history. Nothing secret is in it.")
            }
        }
        .navigationTitle("Settings")
        .navigationBarTitleDisplayMode(.inline)
        .task {
            info = try? await core.run { $0.describe() }
            relay = (try? await core.run { $0.defaultRelay() }) ?? ""
        }
        .sheet(item: $export) { e in
            NavigationStack {
                ScrollView { Text(e.text).font(.footnote.monospaced()).padding().textSelection(.enabled) }
                    .navigationTitle("History")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) { Button("Close") { export = nil } }
                        ToolbarItem(placement: .primaryAction) { ShareLink(item: e.text) }
                    }
            }
        }
        .confirmationDialog("Remove \(removing?.name ?? "this circle") from this phone? Other members keep theirs. This phone's seat key for it is deleted.",
                            isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
                            titleVisibility: .visible) {
            Button("Remove circle", role: .destructive) {
                guard let c = removing else { return }
                Task {
                    do { try await app.delete(c.id) } catch { self.error = plain(error) }
                }
            }
        }
    }

    private func saveRelay() async {
        do {
            try await core.run { [relay] in try $0.setDefaultRelay(url: relay) }
            relay = (try? await core.run { $0.defaultRelay() }) ?? relay
            relayError = nil
            relayNote = "Saved. New circles use this relay."
        } catch {
            relayNote = nil
            relayError = plain(error)
        }
    }

    private func exportHistory(_ c: CircleSummary) async {
        do {
            let text = try await core.run { try $0.exportHistory(circle: c.id) }
            export = ExportText(text: text)
        } catch {
            self.error = plain(error)
        }
    }
}

struct ExportText: Identifiable {
    let id = UUID()
    let text: String
}
