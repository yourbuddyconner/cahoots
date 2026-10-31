import SwiftUI

/// The roster: every seat, the tier it declared, and hello words with each.
struct PeopleView: View {
    let core: Core
    let circle: String
    @State private var members: [Member] = []
    @State private var faces: [UInt8: Data] = [:]
    @State private var error: String?

    static func tier(_ t: UInt8) -> String {
        switch t {
        case 0: return "Tier 0, software only"
        case 1: return "Tier 1, an open microcontroller"
        case 2: return "Tier 2, a sealed microcontroller"
        case 3: return "Tier 3, a hardware key store"
        case 4: return "Tier 4, inspected hardware"
        default: return "Tier \(t), unknown to this app"
        }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xl) {
                Text("To check someone on a call, open their seat and compare hello words.")
                    .foregroundStyle(Color.muted)
                    .fixedSize(horizontal: false, vertical: true)
                PaperSection(footer: "Each tier is what that device declared at the ceremony. It is a claim, not a proof. Every member holds the circle's secret, so the least protected member limits the circle's protection.") {
                    ForEach(Array(members.enumerated()), id: \.element.seat) { i, m in
                        if i > 0 { Hairline().padding(.leading, 44 + Space.m) }
                        if m.mine {
                            row(m)
                        } else {
                            NavigationLink(value: Route.hello(circle, m.seat)) { row(m) }
                                .buttonStyle(RowButtonStyle())
                        }
                    }
                }
                Text("Seat numbers name devices in the ceremony. They are not proof of identity.")
                    .font(.footnote).foregroundStyle(Color.muted)
                StatusLine(text: error)
            }
            .padding(.horizontal, Space.l)
            .padding(.top, Space.s)
            .padding(.bottom, Space.xl)
        }
        .paperBackground()
        .navigationTitle("People")
        .navigationBarTitleDisplayMode(.large)
        .task {
            do {
                members = try await core.run { [circle] in try $0.circleMembers(circle: circle) }
                faces = try await core.run { [circle, members] in
                    $0.faces(circle: circle, members: UInt8(members.count))
                }
            } catch {
                self.error = plain(error)
            }
        }
    }

    private func row(_ m: Member) -> some View {
        HStack(spacing: Space.m) {
            FaceView(pixels: faces[m.seat], size: 44, label: "Seat \(m.seat + 1)'s face")
            VStack(alignment: .leading, spacing: 2) {
                Text(m.mine ? "Seat \(m.seat + 1), you" : "Seat \(m.seat + 1)")
                    .font(.headline).foregroundStyle(Color.ink)
                Text(m.mine
                     ? "\(Self.tier(m.tier)). \(core.signer.usesEnclave ? "The seat key is in this phone's Secure Enclave." : "The seat key is in this simulator's keychain.")"
                     : "\(Self.tier(m.tier)).")
                    .font(.subheadline).foregroundStyle(Color.muted)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: Space.s)
            if !m.mine {
                Text("Hello words")
                    .font(.subheadline)
                    .foregroundStyle(Color.brand)
            }
        }
        .padding(.vertical, Space.m)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }
}

/// Hello words with one other seat: two words each to say on a call.
struct HelloView: View {
    let core: Core
    let circle: String
    let seat: UInt8
    @State private var words: HelloWords?
    @State private var face: Data?
    @State private var error: String?
    @State private var notice: String?

    private var who: String { "Seat \(seat + 1)" }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xl) {
                HStack(spacing: Space.m) {
                    FaceView(pixels: face, size: 44, label: "\(who)'s face")
                    Text("Say yours. If \(who.lowercased()) answers with theirs, it is really them.")
                        .font(.body).foregroundStyle(Color.ink)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if let w = words {
                    VStack(alignment: .leading, spacing: 0) {
                        Hairline()
                        said("You say", w.mine)
                        Hairline()
                        said("\(who) says back", w.theirs)
                        Hairline()
                    }
                    Text("Hello number \(w.number + 1).")
                        .font(.subheadline.monospacedDigit()).foregroundStyle(Color.muted)
                    Text("Only your two devices can make these words. They come from a key that you and \(who.lowercased()) made together during the ceremony. A voice clone cannot say them. Each hello number has its own words, and every word is said once.")
                        .font(.footnote).foregroundStyle(Color.muted)
                        .fixedSize(horizontal: false, vertical: true)
                    Text("Something feel off? Both of you choose Next hello. If they cannot say the new words, that is not them.")
                        .font(.footnote).foregroundStyle(Color.muted)
                        .fixedSize(horizontal: false, vertical: true)
                }
                StatusLine(text: notice, problem: false)
                StatusLine(text: error)
            }
            .padding(.horizontal, Space.l)
            .padding(.top, Space.s)
            .padding(.bottom, Space.xl)
        }
        .paperBackground()
        .navigationTitle("Hello words")
        .navigationBarTitleDisplayMode(.large)
        .actionBar {
            Button("Next hello") { Task { await next() } }
                .buttonStyle(.outlined)
                .disabled(words == nil)
        }
        .task {
            do {
                face = try? await core.run { [circle, seat] in $0.seatFace(circle: circle, seat: seat) }
                words = try await core.run { [circle, seat] in try $0.helloWords(circle: circle, otherSeat: seat) }
            } catch {
                self.error = plain(error)
            }
        }
    }

    private func said(_ label: String, _ word: String) -> some View {
        VStack(alignment: .leading, spacing: Space.xs) {
            Text(label).font(.footnote.weight(.semibold)).foregroundStyle(Color.muted)
            WordView(number: nil, word: word, style: .largeTitle)
        }
        .padding(.vertical, Space.l)
        .accessibilityElement(children: .combine)
    }

    private func next() async {
        do {
            let w = try await core.run { [circle, seat] in try $0.nextHello(circle: circle, otherSeat: seat) }
            words = w
            notice = "Now on hello number \(w.number + 1). Make sure \(who.lowercased()) moves too."
        } catch {
            self.error = plain(error)
        }
    }
}
