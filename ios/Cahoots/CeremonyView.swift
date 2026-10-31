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
            .paperBackground()
            .navigationTitle(model.state?.name ?? "Ceremony")
            .navigationBarTitleDisplayMode(.inline)
            .navigationBarBackButtonHidden(true)
            .toolbar {
                if !model.finished {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("Leave") { confirmLeave = true }
                    }
                }
            }
            .confirmationDialog("Leave setup? This phone’s progress will be discarded. This setup cannot be resumed. Ask everyone to start a new ceremony.", isPresented: $confirmLeave, titleVisibility: .visible) {
                Button("Leave the ceremony", role: .destructive) { Task { await leave() } }
            }
            .safeAreaInset(edge: .bottom) {
                if model.signing {
                    FaceIDBanner().padding(.horizontal, Space.l).padding(.bottom, Space.s).background(Color.paper)
                } else if model.signRefused {
                    Button("Sign with Face ID") { Task { await model.retrySign() } }
                        .buttonStyle(.filled)
                        .padding(.horizontal, Space.l)
                        .padding(.bottom, Space.s)
                        .background(Color.paper)
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
            EndedView(headline: "This setup session has ended", detail: gone) { await leave() }
        } else if let s = model.state {
            if s.seatLost {
                EndedView(headline: "Seat \(s.mySeat + 1) was taken by someone else.",
                          detail: "Leave and join again with a free seat.", button: "Leave") { await leave() }
            } else if s.phase == .aborted {
                EndedView(headline: "The code check failed. Setup has stopped.",
                          detail: s.throughRelay
                          ? "This circle was not saved. Someone outside the group may have the setup code. Ask everyone to start a new ceremony."
                          : "The exchanged data didn’t match. This circle was not saved. Ask everyone to start a new ceremony.") { await leave() }
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
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func leave() async {
        await model.discard()
        app.leave()
    }
}

// MARK: - The frame every step shares

/// Four short segments, blue up to the current step.
struct StepBar: View {
    let step: Int
    var of = 4
    let name: String

    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            HStack(spacing: Space.xs) {
                ForEach(1...of, id: \.self) { i in
                    Capsule()
                        .fill(i <= step ? Color.brand : Color.line)
                        .frame(height: 3)
                }
            }
            Text("Step \(step) of \(of). \(name)")
                .font(.footnote.weight(.medium))
                .foregroundStyle(Color.muted)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Step \(step) of \(of). \(name)")
    }
}

/// A ceremony step: where it is, what to do, then the step's own content.
struct CeremonyStep<Content: View>: View {
    let step: Int
    let stepName: String
    let headline: String
    var detail: String?
    @ViewBuilder var content: Content

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xl) {
                StepBar(step: step, name: stepName)
                ScreenHeader(title: headline, detail: detail, style: .title)
                content
            }
            .padding(.horizontal, Space.l)
            .padding(.top, Space.l)
            .padding(.bottom, Space.xl)
        }
    }
}

// MARK: - Through the relay

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
        CeremonyStep(step: step, stepName: stepName, headline: headline, detail: detail) {
            if !host && state.hostMismatch {
                StatusLine(text: "Seat 1 on the relay is not the host whose code you scanned. Someone outside the room may have the code.")
            }
            if host && commit && !state.needsContinue, let offer = model.offer {
                QRCodeView(matrix: offer)
            }
            if let code = model.joinCode {
                QRCodeView(matrix: code, caption: "You are seat \(me + 1) of \(state.members).")
            }
            if host && (commit || state.needsContinue) {
                seatList
            } else if model.joinCode == nil {
                HStack(spacing: Space.m) {
                    ProgressView()
                    Text(waiting).font(.subheadline).foregroundStyle(Color.muted)
                }
            }
            StatusLine(text: model.notice, problem: false)
            StatusLine(text: model.status)
        }
        .actionBar(shown: state.needsContinue || (host && commit)) {
            if state.needsContinue {
                Button("Continue") { Task { await model.proceed() } }
                    .buttonStyle(.filled)
                    .accessibilityIdentifier("continue")
            } else if host && commit {
                Button("Scan a joiner's code") { scanning = true }
                    .buttonStyle(.filled)
                    .accessibilityIdentifier("scan-joiner")
            }
        }
        .sheet(isPresented: $scanning) {
            NavigationStack {
                ScrollView {
                    VStack(alignment: .leading, spacing: Space.l) {
                        ScanPanel(core: model.core, hint: "Fill the frame with their code",
                                  placeholder: "cahoots:joined:v1:...") { text, pasted in
                            Task {
                                await model.scan(text, pasted: pasted)
                                if model.status == nil { scanning = false }
                            }
                        }
                        StatusLine(text: model.status)
                    }
                    .padding(Space.l)
                }
                .paperBackground()
                .navigationTitle("Scan a joiner's code")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) { Button("Close") { scanning = false } }
                }
            }
        }
    }

    private var step: Int {
        // Until the host has scanned this seat, the joiner is still meeting the group.
        if state.needsContinue || model.joinCode != nil { return 1 }
        switch state.phase {
        case .commit: return 1
        case .reveal: return 2
        default: return 3
        }
    }

    private var stepName: String {
        switch step {
        case 1: return "Meet the group"
        case 2: return "Exchange secrets"
        default: return "Confirm members"
        }
    }

    private var headline: String {
        if state.needsContinue { return "Everyone is scanned" }
        switch state.phase {
        case .commit:
            if host { return "Show this code to everyone joining" }
            return model.joinCode != nil ? "Show this to the host" : "Wait here while everyone joins"
        case .reveal:
            return model.joinCode != nil ? "Show this to the host" : "Exchanging secrets"
        default:
            return "Confirming who is in the circle"
        }
    }

    private var detail: String? {
        if state.needsContinue { return "You’ve scanned everyone’s code. Check that everyone is ready, then tap Continue." }
        switch state.phase {
        case .commit:
            if host { return "Then scan the code on each of their screens." }
            return model.joinCode != nil ? "The host scans it to let you in." : nil
        case .reveal:
            if model.joinCode != nil { return "The host scans it to let you in." }
            return "The phones are exchanging encrypted setup data over the internet. Keep Cahoots open."
        default:
            return "The phones are checking the circle’s member list. Keep Cahoots open."
        }
    }

    private var waiting: String {
        switch state.phase {
        case .commit: return "\(Int(state.members) - state.missing.count) of \(state.members) here."
        case .reveal: return "Waiting for the others."
        default: return "Waiting for every signature."
        }
    }

    /// Every seat and where it is, for the host.
    private var seatList: some View {
        let ok = Set(state.scanned.seats)
        let seen = Set(state.unscanned.seats)
        let missing = Set(state.missing.seats)
        return PaperSection(title: "Seats",
                            footer: seen.isEmpty ? nil : "Scan the code of each person in the room. If you don’t recognize a device that has joined, stop and start a new ceremony.") {
            ForEach(0..<Int(state.members), id: \.self) { i in
                let seat = UInt8(i)
                if i > 0 { Hairline() }
                SeatRow(number: i + 1,
                        state: i == 0 ? .you : ok.contains(seat) ? .scanned
                            : (seen.contains(seat) || (!missing.contains(seat) && !commit)) ? .here : .waiting)
            }
        }
    }
}

/// One seat in the host's list.
struct SeatRow: View {
    enum Where { case you, waiting, here, scanned }
    let number: Int
    let state: Where
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        HStack(spacing: Space.m) {
            Image(systemName: symbol)
                .font(.title3)
                .foregroundStyle(state == .scanned || state == .you ? Color.brand : state == .here ? Color.ink : Color.muted)
                .contentTransition(.symbolEffect(.replace))
                .frame(width: 28)
                .accessibilityHidden(true)
            Text("Seat \(number)").font(.body).foregroundStyle(Color.ink)
            Spacer()
            Text(words)
                .font(.subheadline)
                .foregroundStyle(state == .here ? Color.ink : Color.muted)
        }
        .padding(.vertical, Space.m)
        .frame(minHeight: 44)
        .animation(reduceMotion ? nil : .snappy, value: state)
        .accessibilityElement(children: .combine)
    }

    private var symbol: String {
        switch state {
        case .you: return "person.crop.circle"
        case .waiting: return "circle.dotted"
        case .here: return "circle"
        case .scanned: return "checkmark.circle.fill"
        }
    }

    private var words: String {
        switch state {
        case .you: return "You, the host"
        case .waiting: return "Waiting"
        case .here: return "Here, not scanned"
        case .scanned: return "Scanned"
        }
    }
}

// MARK: - Codes only

/// Codes only: each step, show this seat's code and scan everyone else's.
struct CodesCeremonyView: View {
    @ObservedObject var model: CeremonyModel
    let state: CeremonyState
    @State private var showing = true

    private static let names: [CeremonyPhase: String] = [
        .commit: "Meet the group",
        .reveal: "Exchange secrets",
        .attest: "Confirm members",
    ]

    private var me: UInt8 { state.mySeat }
    private var left: [UInt8] { state.missing.seats.filter { $0 != me } }

    var body: some View {
        CeremonyStep(step: stepNumber, stepName: Self.names[state.phase] ?? "", headline: taskTitle, detail: taskDetail) {
            Picker("Show or scan", selection: $showing) {
                Text(state.phase == .reveal ? "Show my step 2 code" : "Show my code").tag(true)
                Text(state.phase == .reveal ? "Scan step 2 codes" : "Scan their codes").tag(false)
            }
            .pickerStyle(.segmented)
            if showing {
                CodeAnimation(frames: model.frames, caption: "You are seat \(me + 1).")
            } else {
                ScanPanel(core: model.core, hint: "Fill the frame with their animation",
                          placeholder: "cahoots-...") { text, pasted in
                    Task { await model.scan(text, pasted: pasted) }
                }
                if state.framesNeeded > 0 {
                    VStack(alignment: .leading, spacing: Space.s) {
                        ProgressView(value: Double(min(state.framesRead, state.framesNeeded)),
                                     total: Double(state.framesNeeded))
                            .tint(Color.brand)
                        Text("Keep the animated code in view until the progress bar fills.")
                            .font(.footnote).foregroundStyle(Color.muted)
                    }
                }
            }
            Text("Received \(Int(state.members) - 1 - left.count) of \(state.members - 1) \(noun).")
                .font(.subheadline).foregroundStyle(Color.muted)
            StatusLine(text: model.status)
        }
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
        case .reveal: return "step 2 codes"
        default: return "signatures"
        }
    }

    private var who: String {
        let seats = left.map { "seat \($0 + 1)" }
        if seats.count <= 1 { return seats.first ?? "" }
        return seats.dropLast().joined(separator: ", ") + " and " + seats.last!
    }

    private var taskTitle: String {
        if left.isEmpty { return state.phase == .reveal ? "Your turn to show" : "Ready for the next step" }
        switch state.phase {
        case .commit: return "Meet the other devices"
        case .reveal: return "Exchange step 2 codes"
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
        case .reveal: return "Ask \(who) to show their step 2 codes. Scan each animation until the bar fills."
        default: return "Scan signatures from \(who) to confirm the circle's members. They also need to scan yours."
        }
    }
}

/// Codes only: every signature is in here, but the others may still need this seat's.
struct SignatureHandoffView: View {
    @ObservedObject var model: CeremonyModel
    let state: CeremonyState

    var body: some View {
        let others = (0..<Int(state.members)).filter { $0 != Int(state.mySeat) }.map { "seat \($0 + 1)" }
        let list = others.count <= 1 ? (others.first ?? "") : others.dropLast().joined(separator: ", ") + " and " + others.last!
        CeremonyStep(step: 3, stepName: "Confirm members", headline: "Keep this code on screen",
                     detail: "You have every signature. Before comparing words, make sure \(list) \(state.members > 2 ? "have" : "has") scanned yours too.") {
            CodeAnimation(frames: model.frames, caption: "You are seat \(state.mySeat + 1).")
        }
        .actionBar {
            Button(state.members > 2 ? "Everyone has scanned it" : "They have scanned it") { model.handedOff = true }
                .buttonStyle(.filled)
        }
    }
}

// MARK: - The six words

/// Words set large in the serif, numbered in blue. The six check words and hello words use it.
struct WordView: View {
    let number: Int?
    let word: String
    var style: Font.TextStyle = .title

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Space.s) {
            if let number {
                Text("\(number)")
                    .font(.footnote.monospacedDigit().weight(.medium))
                    .foregroundStyle(Color.brand)
                    .frame(minWidth: 14, alignment: .trailing)
            }
            Text(word)
                .serifTitle(style)
                .lineLimit(1)
                .minimumScaleFactor(0.6)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// The six words, read aloud together.
struct WordsView: View {
    @EnvironmentObject private var app: AppModel
    @ObservedObject var model: CeremonyModel
    let state: CeremonyState
    @State private var confirmDiffer = false
    @State private var keeping = false
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        CeremonyStep(step: 4, stepName: "Compare words", headline: "Compare all six words",
                     detail: "One person reads aloud while everyone checks their own screen. Confirm only when all six words match on every phone, in the same order. If any differ, everyone starts again.") {
            words
            StatusLine(text: model.status)
        }
        .actionBar {
            Button {
                Task {
                    keeping = true
                    defer { keeping = false }
                    if let id = await model.keep() { await app.kept(id) }
                }
            } label: {
                Working(busy: keeping, title: "They match")
            }
            .buttonStyle(.filled)
            .disabled(keeping)
            .accessibilityLabel("They match")
            Button("They differ") { confirmDiffer = true }
                .buttonStyle(.text)
        }
        .sensoryFeedback(.success, trigger: keeping) { _, now in now }
        .confirmationDialog("Discard this setup and start again?", isPresented: $confirmDiffer, titleVisibility: .visible) {
            Button("Discard setup", role: .destructive) {
                Task {
                    await model.discard()
                    app.leave()
                }
            }
        }
    }

    private var words: some View {
        let w = state.words
        let half = (w.count + 1) / 2
        let column = { (range: Range<Int>) in
            VStack(alignment: .leading, spacing: Space.l) {
                ForEach(range, id: \.self) { i in WordView(number: i + 1, word: w[i]) }
            }
        }
        return VStack(spacing: 0) {
            Hairline()
            Group {
                if typeSize.isAccessibilitySize {
                    column(0..<w.count)
                } else {
                    HStack(alignment: .top, spacing: Space.l) {
                        column(0..<half)
                        column(half..<w.count)
                    }
                }
            }
            .padding(.vertical, Space.xl)
            Hairline()
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(w.enumerated().map { "\($0.offset + 1) \($0.element)" }.joined(separator: ", "))
        .accessibilityIdentifier("words")
    }
}

/// A ceremony that ended without a circle.
struct EndedView: View {
    let headline: String
    let detail: String
    var button = "Back to start"
    let action: () async -> Void

    var body: some View {
        ScrollView {
            ScreenHeader(title: headline, detail: detail, style: .title)
                .padding(.horizontal, Space.l)
                .padding(.top, Space.xxl)
        }
        .actionBar {
            Button(button) { Task { await action() } }
                .buttonStyle(.filled)
        }
    }
}
