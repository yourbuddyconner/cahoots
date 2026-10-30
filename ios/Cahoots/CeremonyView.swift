import SwiftUI

/// A ceremony, start to finish: the codes, the relay's progress, then the six words.
struct CeremonyView: View {
    @EnvironmentObject private var app: AppModel
    @StateObject private var model: CeremonyModel
    @State private var confirmLeave = false

    init(core: Core, id: String) {
        _model = StateObject(wrappedValue: CeremonyModel(core: core, id: id))
    }

    var body: some View {
        content
            .navigationBarBackButtonHidden(true)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if !model.finished {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("Leave") { confirmLeave = true }
                    }
                }
            }
            .confirmationDialog("Leave the ceremony? Nothing is kept.", isPresented: $confirmLeave, titleVisibility: .visible) {
                Button("Leave the ceremony", role: .destructive) { Task { await leave() } }
            }
            .safeAreaInset(edge: .bottom) {
                if model.signing {
                    FaceIDBanner().padding()
                } else if model.signRefused {
                    Button("Sign with Face ID") { Task { await model.retrySign() } }
                        .buttonStyle(.borderedProminent)
                        .padding()
                }
            }
            .task {
                await model.load()
                if model.state?.throughRelay == true { await model.run() }
            }
    }

    @ViewBuilder
    private var content: some View {
        if let gone = model.gone {
            EndedView(title: "Ceremony lost", headline: "This ceremony is gone.", detail: gone) { await leave() }
        } else if let s = model.state {
            if s.seatLost {
                EndedView(title: "Seat taken", headline: "Seat \(s.mySeat + 1) was taken by someone else.",
                          detail: "Leave and join again with a free seat.", button: "Leave") { await leave() }
            } else if s.phase == .aborted {
                EndedView(title: "Ceremony stopped", headline: "A reveal did not match its commitment.",
                          detail: s.throughRelay
                          ? "Nothing was kept. Someone outside the room may have the code. Everyone starts over with a new code."
                          : "Nothing was kept. Either someone's contribution changed after they committed, or a code was corrupted. Start again from the beginning.") { await leave() }
            } else if s.phase == .complete && (s.throughRelay || model.handedOff) {
                WordsView(model: model, state: s)
            } else if s.throughRelay {
                RelayCeremonyView(model: model, state: s)
            } else if s.phase == .complete {
                SignatureHandoffView(model: model, state: s)
            } else {
                CodesCeremonyView(model: model, state: s)
            }
        } else {
            ProgressView()
        }
    }

    private func leave() async {
        await model.discard()
        app.leave()
    }
}

/// Seat chips: you, and who is in.
struct SeatChips: View {
    let members: UInt8
    let me: UInt8
    /// Seats still missing, from 0.
    let missing: Set<UInt8>

    var body: some View {
        HStack(spacing: 6) {
            ForEach(0..<Int(members), id: \.self) { i in
                let seat = UInt8(i)
                let mine = seat == me
                let inHere = !missing.contains(seat)
                Text(mine ? "you · \(i + 1)" : inHere ? "\(i + 1) ✓" : "\(i + 1)")
                    .font(.footnote.weight(.medium))
                    .padding(.horizontal, 10)
                    .padding(.vertical, 4)
                    .background(mine ? Color.accentColor.opacity(0.2) : inHere ? Color.green.opacity(0.2) : Color.secondary.opacity(0.12),
                                in: Capsule())
            }
        }
    }
}

/// Through the relay: the host shows one code and scans each joiner, the joiner shows its code to
/// the host, and the rest goes through the relay.
struct RelayCeremonyView: View {
    @ObservedObject var model: CeremonyModel
    let state: CeremonyState
    @State private var scanning = false

    private var me: UInt8 { state.mySeat }
    private var host: Bool { me == 0 }
    private var commit: Bool { state.phase == .commit }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text("\(state.name) · you are seat \(me + 1)").font(.footnote).foregroundStyle(.secondary)
                SeatChips(members: state.members, me: me, missing: commit ? Set(state.missing.seats) : [])
                Text(commit ? "\(Int(state.members) - state.missing.count) of \(state.members) here"
                            : "Everyone is here · \(state.members) of \(state.members)")
                    .font(.footnote).foregroundStyle(.secondary)
                Text(step).font(.headline)
                if !host && state.hostMismatch {
                    StatusLine(text: "Seat 1 on the relay is not the host whose code you scanned. Someone outside the room may have the code.")
                }
                if host && commit, let offer = model.offer {
                    QRCodeView(matrix: offer, caption: "\(state.name) · join")
                    scannedList
                }
                if let code = model.joinCode {
                    QRCodeView(matrix: code, caption: "seat \(me + 1) · for the host")
                }
                StatusLine(text: model.notice, problem: false)
                StatusLine(text: model.status)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding()
        }
        .navigationTitle(title)
        .safeAreaInset(edge: .bottom) {
            VStack {
                if state.needsContinue {
                    Button("Continue") { Task { await model.proceed() } }
                        .buttonStyle(.borderedProminent)
                        .accessibilityIdentifier("continue")
                } else if host && commit {
                    Button("Scan a joiner's code") { scanning = true }
                        .buttonStyle(.borderedProminent)
                        .accessibilityIdentifier("scan-joiner")
                }
            }
            .frame(maxWidth: .infinity)
            .padding(.bottom, 8)
        }
        .sheet(isPresented: $scanning) {
            NavigationStack {
                ScrollView {
                    ScanPanel(core: model.core, hint: "Fill the frame with their code",
                              placeholder: "cahoots:joined:v1:...") { text, pasted in
                        Task {
                            await model.scan(text, pasted: pasted)
                            if model.status == nil { scanning = false }
                        }
                    }
                    .padding()
                    StatusLine(text: model.status).padding(.horizontal)
                }
                .navigationTitle("Scan a joiner's code")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) { Button("Close") { scanning = false } }
                }
            }
        }
    }

    private var title: String {
        if state.needsContinue { return "Everyone is scanned" }
        if host && commit { return "Show this code to everyone joining" }
        if model.joinCode != nil { return "Show this to the host" }
        return "Through the relay"
    }

    private var step: String {
        if state.needsContinue { return "Everyone is scanned." }
        switch state.phase {
        case .commit:
            if host { return "Show this code to everyone joining. Then scan the code on each of their screens." }
            return model.joinCode != nil ? "Show this to the host." : "Wait here while everyone joins."
        case .reveal:
            return model.joinCode != nil ? "Show this to the host." : "Exchanging secrets through the relay."
        default:
            return "Confirming who is in the circle."
        }
    }

    private var scannedList: some View {
        let ok = Set(state.scanned.seats)
        let seen = Set(state.unscanned.seats)
        return VStack(alignment: .leading, spacing: 6) {
            ForEach(1..<Int(state.members), id: \.self) { i in
                let seat = UInt8(i)
                Text(ok.contains(seat) ? "Seat \(i + 1) · scanned ✓"
                     : seen.contains(seat) ? "Seat \(i + 1) committed but was not scanned here."
                     : "Seat \(i + 1) · waiting")
                    .font(.subheadline)
                    .foregroundStyle(seen.contains(seat) ? Color.orange : Color.primary)
            }
        }
    }
}

/// Codes only: each step, show this seat's code and scan everyone else's.
struct CodesCeremonyView: View {
    @ObservedObject var model: CeremonyModel
    let state: CeremonyState
    @State private var showing = true

    private static let steps: [CeremonyPhase: String] = [
        .commit: "Step 1 of 4 · Meet the group",
        .reveal: "Step 2 of 4 · Exchange secrets",
        .attest: "Step 3 of 4 · Confirm members",
    ]

    private var me: UInt8 { state.mySeat }
    private var left: [UInt8] { state.missing.seats.filter { $0 != me } }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text("\(state.name) · you are seat \(me + 1)").font(.footnote).foregroundStyle(.secondary)
                SeatChips(members: state.members, me: me, missing: Set(state.missing.seats))
                VStack(alignment: .leading, spacing: 4) {
                    Text(taskTitle).font(.headline)
                    Text(taskDetail).foregroundStyle(.secondary)
                }
                Picker("Mode", selection: $showing) {
                    Text(state.phase == .reveal ? "Show my reveal" : "Show my code").tag(true)
                    Text(state.phase == .reveal ? "Scan their reveals" : "Scan their codes").tag(false)
                }
                .pickerStyle(.segmented)
                if showing {
                    CodeAnimation(frames: model.frames, caption: "seat \(me + 1) · step \(stepNumber)")
                } else {
                    ScanPanel(core: model.core, hint: "Fill the frame with their animation",
                              placeholder: "cahoots-...") { text, pasted in
                        Task { await model.scan(text, pasted: pasted) }
                    }
                    if state.framesNeeded > 0 {
                        ProgressView(value: Double(min(state.framesRead, state.framesNeeded)),
                                     total: Double(state.framesNeeded))
                        Text("Hold steady on the animation. The bar fills as frames arrive.")
                            .font(.footnote).foregroundStyle(.secondary)
                    }
                }
                Text("Received \(Int(state.members) - 1 - left.count) of \(state.members - 1) \(noun)")
                    .font(.footnote).foregroundStyle(.secondary)
                StatusLine(text: model.status)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding()
        }
        .navigationTitle(Self.steps[state.phase] ?? "")
    }

    private var stepNumber: Int {
        switch state.phase {
        case .commit: return 1
        case .reveal: return 2
        default: return 3
        }
    }

    private var noun: String {
        switch state.phase {
        case .commit: return "setup codes"
        case .reveal: return "reveals"
        default: return "signatures"
        }
    }

    private var who: String {
        left.map { "seat \($0 + 1)" }.joined(separator: ", ")
    }

    private var taskTitle: String {
        if left.isEmpty { return state.phase == .reveal ? "Your turn to show" : "Ready for the next step" }
        switch state.phase {
        case .commit: return "Meet the other devices"
        case .reveal: return "Collect their secret contributions"
        default: return "Confirm who is in the circle"
        }
    }

    private var taskDetail: String {
        if left.isEmpty {
            return state.phase == .reveal
                ? "Ask everyone to scan this code. Wait until they confirm before continuing."
                : "You have everything for this step."
        }
        switch state.phase {
        case .commit: return "Scan the setup codes from \(who). They also need to scan yours. You can take turns in any order."
        case .reveal: return "Ask \(who) to show their step-2 codes. Scan each animation until the progress bar fills."
        default: return "Scan signatures from \(who) to confirm the group's members. They also need to scan yours."
        }
    }
}

/// Codes only: every signature is in here, but the others may still need this seat's.
struct SignatureHandoffView: View {
    @ObservedObject var model: CeremonyModel
    let state: CeremonyState

    var body: some View {
        let others = (0..<Int(state.members)).filter { $0 != Int(state.mySeat) }.map { "seat \($0 + 1)" }
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text("You have every signature. Before comparing words, make sure \(others.joined(separator: " and ")) \(state.members > 2 ? "have" : "has") scanned yours too. Keep this code on screen until they confirm.")
                CodeAnimation(frames: model.frames, caption: "seat \(state.mySeat + 1) · step 3")
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding()
        }
        .navigationTitle("Step 3 of 4 · Confirm members")
        .safeAreaInset(edge: .bottom) {
            Button(state.members > 2 ? "Everyone has scanned it" : "They have scanned it") { model.handedOff = true }
                .buttonStyle(.borderedProminent)
                .padding(.bottom, 8)
        }
    }
}

/// The six words, read aloud together.
struct WordsView: View {
    @EnvironmentObject private var app: AppModel
    @ObservedObject var model: CeremonyModel
    let state: CeremonyState
    @State private var confirmDiffer = false
    @State private var keeping = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text("Read all six words aloud together. Check your own screen: every word must match, in the same order. If one differs, everyone starts again.")
                VStack(alignment: .leading, spacing: 4) {
                    Text(state.name).font(.footnote).foregroundStyle(.secondary)
                    LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], alignment: .leading, spacing: 12) {
                        ForEach(Array(state.words.enumerated()), id: \.offset) { i, w in
                            HStack(alignment: .firstTextBaseline, spacing: 8) {
                                Text("\(i + 1)").font(.caption).foregroundStyle(.secondary)
                                Text(w).font(.title3.weight(.semibold))
                            }
                        }
                    }
                    .padding()
                    .background(Color.secondary.opacity(0.08), in: RoundedRectangle(cornerRadius: 12))
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("words")
                }
                StatusLine(text: model.status)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding()
        }
        .navigationTitle("Step 4 of 4 · Compare words")
        .safeAreaInset(edge: .bottom) {
            VStack(spacing: 10) {
                Button {
                    Task {
                        keeping = true
                        defer { keeping = false }
                        if let id = await model.keep() { await app.kept(id) }
                    }
                } label: {
                    Text("They match").frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .disabled(keeping)
                Button("They differ", role: .destructive) { confirmDiffer = true }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding()
        }
        .confirmationDialog("Throw this away and start again?", isPresented: $confirmDiffer, titleVisibility: .visible) {
            Button("Throw it away", role: .destructive) {
                Task {
                    await model.discard()
                    app.leave()
                }
            }
        }
    }
}

/// A ceremony that ended without a circle.
struct EndedView: View {
    let title: String
    let headline: String
    let detail: String
    var button = "Back to start"
    let action: () async -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(headline).font(.title2.weight(.semibold))
            Text(detail).foregroundStyle(.secondary)
            Spacer()
            Button(button) { Task { await action() } }
                .buttonStyle(.borderedProminent)
                .frame(maxWidth: .infinity)
        }
        .padding()
        .navigationTitle(title)
    }
}
