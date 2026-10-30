---
title: Privacy Hardware Needs a Renaissance
date: 2026-10-25
author: Conner Swann
role: Building Cahoots
tags: Building in public, Privacy
about: Why I'm building Cahoots. A circle should be able to establish its own trust, decide who can act on its behalf, and carry that authority on hardware its members own.
audience: People who want more control over their private lives, and the hardware and software builders who want to help.
learn:
- What it means for a group to own its trust
- Where Cahoots fits alongside existing privacy tools
- What a circle should let you do after meeting in person
- How agents can act for a circle under explicit, limited grants
- Why the badge matters, and what it cannot promise
description: The people you trust should be able to make a private circle, set the terms for agents acting on their behalf, and carry that authority on hardware they own. That's why I'm building Cahoots.
---

I care about privacy. I have for a long time. But caring about it mostly meant choosing the right tools and hoping the people behind them kept doing their jobs. A good messaging app. A password manager. A VPN. I could make those choices and get on with my life.

The recent certificate-authority failures made me look harder at that arrangement. The quiet machinery underneath a secure connection is people and scripts like everything else. It can be wrong. And when it is, I usually have no say in how it gets fixed and very little ability to keep going without it.

I don't want every private part of my life to be a tenancy. I want some of it to belong to me and the people I share it with.

That's why I'm building Cahoots. And it's why I think privacy hardware needs a renaissance: small, understandable devices that give people real custody of their keys and a meaningful say in what those keys do.

That matters even more as we put software agents to work on our behalf. I want the authority those programs use to start with us, with limits we can understand and enforce.

## The people you already know

The public internet has a hard job. You need to talk to strangers, visit websites you've never seen, and pay people you've never met. That needs systems that work at the scale of strangers. Cahoots isn't going to replace all of them.

But a lot of my life isn't strangers. It's my family, my friends, the people I work with, and the machines we run. We already know who we are. We've sat across a table from each other. We can hand each other a house key.

Why is it still so much work to carry that relationship into a computer?

Every service asks us to establish it again on its terms. Make an account. Find the right person. Invite them. Give the service enough authority to manage the relationship. Do it again in the next app. Eventually a group of people who know each other becomes a collection of accounts whose connection is maintained by a dozen unrelated companies.

Those companies can build good tools. I use them. I also want the group itself to have something durable underneath those tools, something it can carry elsewhere.

The people you'd hand a house key to, you have already met. That should count for more.

## Make a circle, then use it

Cahoots starts with a group meeting in person. Each person's device contributes randomness. The devices exchange encrypted contributions, combine them, and produce a shared secret. The group compares the words on their screens to check that they agree. The ceremony also records the members' public signing keys in a membership record they sign together.

No account provider chooses the members. No certificate authority needs to vouch for the people standing in front of you. No relay gets a copy of the circle's secret.

<figure>
<img src="/docs/diagrams/ceremony.svg" alt="The ceremony in six steps: set up, draw, swap, mix, sign, say the words" width="960" height="400">
<figcaption>The ceremony establishes the circle. The <a href="/docs/#/ceremony-guide">guide</a> explains the steps and what everyone needs to check.</figcaption>
</figure>

But standing around making a secret isn't an ecosystem. The point is what you can do afterward.

I want a circle to be a place you return to. You leave the room and keep talking in its group chat. You can see who belongs to it. You can use its keys to protect a file or help establish a private connection between your machines. When you need to approve something with your badge, you plug it into your computer, read what it's asking, and press the buttons yourself.

The phone and browser belong in that everyday experience. They're where people already read, type, and talk. The badge gives the circle a physical home for keys and an explicit point of approval when those keys are used. It shouldn't take a ceremony every time you want to have a conversation.

The ambition is that making the circle once should make the next private thing easier.

## Who gets to act for us

An agent that can actually do things needs access. It needs to read something, call a service, use a key, or ask another program to do part of the job. Every one of those actions raises a question: who gave it permission?

I think identity is poorly understood partly because we ask it to carry too much. Knowing which key signed a request doesn't tell you whether its owner may make that request. Knowing a person belongs to a group doesn't mean they can authorize anything in the group's name. And a program acting for me should have its own identity, so we can tell what it did and under whose authority.

Delegation has a long history. The web already has standards for [delegated access](https://www.rfc-editor.org/rfc/rfc8693.html) and [more precise permission requests](https://www.rfc-editor.org/rfc/rfc9396.html). Agents make the consequences much harder to ignore. A program can use whatever access you gave it faster, more often, and further down a chain of other programs than you ever would by hand. Handing it your account and hoping it behaves is a miserable foundation for that future.

The circle should give us a firm boundary and a way to authorize useful work across it. A grant should say which key may do what, how much authority it has, when that authority ends, and whether it can pass a smaller part onward. A service checking the request should be able to follow that authority back to the members who granted it. Some actions should always require a person to approve them.

For example, I want an agent to be able to arrange a private connection for a particular job without giving it every secret the circle holds. If it delegates part of the job, that next program should receive only the authority it needs, within the original grant. Sharing a circle's whole secret would give it the access of a member; a limited task deserves a narrower door.

That authority needs to hold all the way through: deliberate consent on the badge, grants that can only narrow as they pass between programs, and services that refuse work outside those limits. We need to be able to withdraw a grant when the job is over or something goes wrong. Giving a program permission should leave us in charge of the terms.

This is the autonomy I want: a group can establish its own authority, choose its rules, and give a machine a bounded job without handing over the power to rewrite those rules. We should be able to check that arrangement without asking the machine whether it followed our instructions.

## Good tools, more control

Signal is good. Tailscale is good. A password manager and a hardware security key are good things to have. Please don't read this as an argument to throw them away.

They also don't all have the same trust model. Signal makes private keys on your device and lets you verify a contact's safety number. Tailscale makes node keys locally, and Tailnet Lock lets your own nodes sign the keys they'll accept. A security key keeps a private key in hardware. These are real ways to reduce how much authority an intermediary has.

My frustration is that putting these protections together still asks a lot of the person using them. Each tool has its own way to establish trust, its own recovery story, and its own boundaries. The relationship you verified in one place doesn't automatically help you in the next.

Cahoots is an attempt to make the group a useful starting point across those boundaries. A shared circle can supply a pre-shared key for a WireGuard tunnel, key material for chat, or a key for encrypting a file. Members also have individual signing keys for actions that need to be attributable to one person.

There will still be infrastructure. Messages need delivery. Software needs updates. Devices break. We run a hosted relay because people should be able to start a conversation without first becoming server administrators. The relay can fail, observe traffic patterns, or refuse to deliver a message. It doesn't need the circle's keys to do its job.

I want to be able to draw that line clearly: here's what the service does, here's what it can learn, and here's what remains with us.

## Why I want a badge

A phone can make and use a circle. It is also a general-purpose computer with a radio, a cloud account, and a few hundred other things going on. A browser is convenient, but the computer running it may be compromised. Putting keys there has consequences.

A badge gives us a chance to make those consequences smaller and more legible. A device with a narrow job. A screen you can read. Buttons you have to press. Keys that can stay in a secure element while the device uses them. Board files and firmware you can inspect, build, and argue with.

I want to plug it into a laptop and have it tell me what I'm approving. I want it to carry several circles without turning them into one public identity. I want it to be useful enough that someone actually keeps it with them.

That includes delegation. I want to read the authority I'm giving a program on a screen it doesn't control, approve it deliberately, and let it get on with the work inside those limits. Having agency shouldn't mean pressing a button for every routine action. It should mean having a real say in which actions can happen without me.

That takes more than declaring a piece of hardware secure. A screen is only useful if it shows enough to make a decision. A button is only useful if the firmware can't quietly go around it. An open board doesn't prove the device in your hand was built correctly. Randomness, manufacturing, updates, theft, and recovery all deserve actual answers.

Hardware wallets and login keys have done important work here. I want to bring that kind of deliberate custody to the private relationships between ordinary groups of people, and make it practical beyond a login screen or a transaction.

There should be more people trying this. More boards. More approaches. More things you can own, fix, and understand well enough to decide whether you trust them.

## The limits matter

Part of what got me here was a pessimistic question: what do we keep if the cryptography we rely on stops holding up?

Cahoots doesn't get to wave that problem away because the people met in a room. Its ceremony encrypts contributions with X-Wing, combining X25519 and ML-KEM-768. Someone who records the exchange is being kept out by that cryptography. If those protections fail, the recording matters.

There is a narrower, useful property: a truly random pad, kept secret and never reused, can protect a message without relying on a hard mathematical problem. Cahoots has a pure-pad message mode for that. Ordinary chat, file encryption, and network integrations use conventional cryptography and keep its limitations. Deriving their keys from a circle doesn't turn them into one-time pads.

A compromised member's device can still expose what it holds. Another member can share a secret you wanted them to keep. A badge doesn't solve coercion. Those limits belong in the explanation, because owning a tool ought to include knowing where it stops helping.

I still think there's something worth building here. We can give a group more control over how it establishes trust, where its keys live, and what it authorizes, while being honest about the assumptions underneath it.

## Privacy as a core competency

I don't want to become a part-time administrator just to have a private life. I don't think anyone else should have to, either. The tools need to do more of the work.

But I do want to be able to answer three questions: Who else can read this? What would they have to do to read it? What happens if they're compromised?

And when a program acts for us: Who authorized it? What can it do? How do we stop it? Those answers need to live in the tools, where the people granting access can see and change them.

Too often the answer is somewhere behind a subscription, a reassuring illustration, and a promise that somebody has it covered. I am tired of how little purchase that gives me on the actual situation. If the answer changes tomorrow, I'd like a choice beyond accepting it or abandoning the people and work I've built there.

For a family, that might mean a private place to talk and keep things together. For a small organization, it might mean keys and connections it can keep using when a vendor has a bad week. Neither should require everyone involved to study cryptography. Both deserve tools that make their choices understandable.

That's the competency I care about: enough understanding, backed by usable tools, to have a say in your own affairs.

## What comes next

I'm building the badge and working out how to manufacture it. The software belongs to the same ambition: a circle should be a place to talk, use our tools, and decide what our programs may do on our behalf.

The next posts will get into the board, the constraints, the mistakes, and the work of connecting these pieces.

If you've built hardware like this, or tried to, I'd like to hear what you ran into. The code and board files are on [GitHub](https://github.com/yourbuddyconner/cahoots). Build one. Tell me what's wrong with it. Help make the next one better.
