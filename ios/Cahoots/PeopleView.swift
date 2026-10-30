import SwiftUI

/// The roster: every seat, the tier it declared, and hello words with each.
struct PeopleView: View {
    let core: Core
    let circle: String
    @State private var members: [Member] = []
    @State private var error: String?

    static func tier(_ t: UInt8) -> String {
        switch t {
        case 0: return "T0 software"
        case 1: return "T1 open microcontroller"
        case 2: return "T2 sealed microcontroller"
        case 3: return "T3 hardware key store"
        case 4: return "T4 inspected hardware"
        default: return "Tier \(t), unknown to this app"
        }
    }

    var body: some View {
        List {
            Section {
                ForEach(members, id: \.seat) { m in
                    if m.mine {
                        row(m)
                    } else {
                        NavigationLink(value: Route.hello(circle, m.seat)) { row(m) }
                    }
                }
            } header: {
                Text("The people in this circle")
            } footer: {
                Text("Each tier is what that device declared at the ceremony. It is a claim, not a proof. Every member holds the circle's secret, so the least protected member limits the circle's protection.")
            }
            Section {
                Text("Seat labels identify devices in the ceremony. They are not proof of identity. To check someone on a call, open their seat and compare hello words.")
                    .font(.footnote).foregroundStyle(.secondary)
            }
            StatusLine(text: error)
        }
        .navigationTitle("People")
        .navigationBarTitleDisplayMode(.inline)
        .task {
            do {
                members = try await core.run { [circle] in try $0.circleMembers(circle: circle) }
            } catch {
                self.error = plain(error)
            }
        }
    }

    private func row(_ m: Member) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(m.mine ? "Seat \(m.seat + 1) · you" : "Seat \(m.seat + 1)").font(.headline)
            Text(m.mine ? "\(Self.tier(m.tier)) · this phone, seat key in the Secure Enclave" : Self.tier(m.tier))
                .font(.footnote).foregroundStyle(.secondary)
        }
    }
}

/// Hello words with one other seat: two words each to say on a call.
struct HelloView: View {
    let core: Core
    let circle: String
    let seat: UInt8
    @State private var words: HelloWords?
    @State private var error: String?
    @State private var notice: String?

    private var who: String { "Seat \(seat + 1)" }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                if let w = words {
                    Text("Say yours. If \(who) answers with theirs, it is really them.").font(.headline)
                    VStack(alignment: .leading, spacing: 10) {
                        Text("hello #\(w.number + 1)").font(.footnote).foregroundStyle(.secondary)
                        Text("You say").font(.footnote).foregroundStyle(.secondary)
                        Text(w.mine).font(.title.weight(.semibold))
                        Text("\(who) says back").font(.footnote).foregroundStyle(.secondary)
                        Text(w.theirs).font(.title.weight(.semibold))
                    }
                    .padding()
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.secondary.opacity(0.08), in: RoundedRectangle(cornerRadius: 12))
                    Text("Only your two devices can make these words: they come from a key that you and \(who.lowercased()) made together during the ceremony. A voice clone cannot say them. Each hello number has its own words, and every word is said once.")
                        .font(.footnote).foregroundStyle(.secondary)
                    Text("Something feel off? Both of you choose Next hello. If they cannot say the new words, that is not them.")
                        .font(.footnote).foregroundStyle(.secondary)
                }
                StatusLine(text: notice, problem: false)
                StatusLine(text: error)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding()
        }
        .navigationTitle("Hello words")
        .navigationBarTitleDisplayMode(.inline)
        .safeAreaInset(edge: .bottom) {
            Button("Next hello") { Task { await next() } }
                .buttonStyle(.borderedProminent)
                .padding(.bottom, 8)
                .disabled(words == nil)
        }
        .task {
            do {
                words = try await core.run { [circle, seat] in try $0.helloWords(circle: circle, otherSeat: seat) }
            } catch {
                self.error = plain(error)
            }
        }
    }

    private func next() async {
        do {
            let w = try await core.run { [circle, seat] in try $0.nextHello(circle: circle, otherSeat: seat) }
            words = w
            notice = "Now on hello #\(w.number + 1). Make sure \(who.lowercased()) moves too."
        } catch {
            self.error = plain(error)
        }
    }
}
