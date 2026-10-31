---
title: Privacy Hardware Needs a Renaissance
date: 2026-10-25
author: Conner Swann
role: Building Cahoots
tags: Building in public, Privacy
about: What would it take for your digital footprint to be truly yours? Why that question led me to circles, identity within our relationships, and hardware you can own and inspect.
audience: People who want more control over their private lives, and the hardware and software builders who want to help.
learn:
- Why choosing good privacy tools still leaves me wanting more
- How a group could carry its trust between tools
- Why each member needs an identity within the circle
- What it means to let a program act on your behalf
- Why those decisions led me to build a badge
- What ownership asks of the hardware, and of the people building it
description: What would it take for your digital footprint to be truly yours? For me, that question starts with the people I trust and leads all the way down to the hardware.
---

I care about privacy. I have for a long time. But caring about it mostly meant choosing the right tools and hoping the people behind them kept doing their jobs. A good messaging app. A password manager. A VPN. I could make those choices and get on with my life.

The certificate-authority failures made me look harder at that arrangement. The quiet machinery underneath a secure connection is people and scripts like everything else. It can be wrong. What bothered me was how little say I had when it was. I could choose another service, but I would be making much the same bargain with someone else.

<figure>
<img src="/blog/img/dependency.jpg" alt="A cartoon tower of blocks labeled all modern digital infrastructure, balanced on a few small pieces at the bottom" width="687" height="826" class="narrow">
<figcaption>All modern digital infrastructure, after <a href="https://xkcd.com/2347/">xkcd 2347</a>. Pick any block and ask who runs it and whether you could tell if they stopped.</figcaption>
</figure>

I started thinking about how much of my life sits inside arrangements like that. Conversations with my family. Work with friends. Files, accounts, and the machines we use together. Years of little decisions that add up to something I would have a hard time taking elsewhere.

What would it take for your digital footprint to be truly yours?

I don't expect to own the internet. I will always depend on other people, their work, and their machines. But I don't want every private part of my life to be a tenancy. I want some of it to belong to me and the people I share it with. I want us to have a say in who gets access, what our programs can do, and what happens when a service changes underneath us.

Following that thought is what led me to Cahoots. Eventually, it led me to a circuit board.

## Start with the people you already know

The public internet has a hard job. You need to talk to strangers, visit websites you've never seen, and pay people you've never met. That needs systems that work at the scale of strangers.

But a lot of my life isn't strangers. It's my family, my friends, the people I work with, and the machines we run. We already know who we are. We've sat across a table from each other. We can hand each other a house key.

Why is it still so much work to carry that relationship into a computer?

Every service asks us to establish it again on its terms. Make an account. Find the right person. Invite them. Do it again in the next app. Eventually a group of people who know each other becomes a collection of accounts whose connection is maintained by a dozen unrelated companies.

There are good tools here. I use Signal and Tailscale. Signal’s [safety numbers](https://support.signal.org/hc/en-us/articles/360007060632-What-is-a-safety-number-and-why-do-I-see-that-it-changed) and Apple’s [iMessage Contact Key Verification](https://security.apple.com/blog/imessage-contact-key-verification) already give people a way to compare codes and check the keys behind a conversation. Meeting someone and checking what appears on both screens is an established pattern. Cahoots builds on it.

The human part matters. Comparing keys doesn't establish who a person is by itself. You bring that knowledge to the exchange: this is the friend sitting across from me, and this is the device they're using. The check connects that relationship to the keys the software will use.

I'd like to extend that pattern beyond checking public keys for a conversation in one app. A group could establish a shared secret and agree on its members together, then carry that foundation into other tools. Each tool would still have its own job and its own limits, but the group wouldn't have to establish the relationship from scratch every time.

The people you'd hand a house key to, you have already met. That should count for more.

## A circle of our own

Imagine meeting up with a few people you trust and getting your devices out to make a private circle together. As the devices exchange information, you compare the words on your screens to check that everyone agrees. By the time you leave, the group has established something it can use beyond that room.

That's the starting point for Cahoots. During the meeting, which we call a ceremony, each device contributes randomness that the devices combine into a shared secret. They also sign a record of who belongs, so membership comes from the people taking part. There's no account provider choosing the members or certificate authority vouching for the people standing in front of you, and the circle's secret stays with its members. The [ceremony guide](/docs/#/ceremony-guide) explains how that works and what everyone needs to check.

What interests me is what happens afterward. When you want to keep talking, protect a file for the group, or connect two of your machines, you should be able to start with the circle you've already made. The group has its own record of membership and its own keys to bring to those tools. Each new use can build on a relationship you've already established, without asking another service to define it for you.

<figure>
<img src="/blog/img/circle-and-tools.svg" alt="People meet in person to establish their circle. The circle supplies a common foundation for chat, files, and private connections, with separate keys for each use." width="640" height="480">
<figcaption>The ambition: establish the circle together, then carry it into the tools you use. Each use gets its own keys.</figcaption>
</figure>

Owning the circle still leaves us needing other people's infrastructure. We run a hosted relay so people can keep talking without first becoming server administrators, but delivering those messages doesn't require the relay to hold the circle's keys. Its operator can still observe traffic patterns or refuse delivery, which is why you should be able to run your own and choose where your conversations travel.

That's what ownership should let us do: use a service while keeping enough in our hands to leave it. I want the group to understand what that service does and what it can learn, and to be able to change it without having to establish the circle all over again.

Cahoots is still a development project, with chat and the browser ceremony available to try while the integrations and hardware are at different stages. The [status page](/docs/#/status) keeps track of that work. The direction I want to take it is simple: making the circle once should make the next private thing easier.

## A place for each of us

A circle also needs a way to recognize the people inside it. Sharing a private conversation should still leave room for each of us to make our own decisions.

The shared secret gives us something we can protect together. Every member holds it. That is useful for keeping a conversation within the group, but it cannot tell us which member approved a decision. If three of us can make the same cryptographic proof, that proof alone cannot distinguish us.

So the ceremony also gives each device its own signing key and records its public key in the circle's membership list. The private key stays with that device. When I sign something, the others can check it against the key we agreed belonged to my seat at the table. The circle can recognize an individual decision as well as a shared secret.

I want that identity to live in the context of the relationship. My key in a work circle is separate from my key in a family circle. Neither needs to become a universal identifier that follows me everywhere. A stolen key for one circle should not automatically give someone authority in another. Separate keys don't make the relationships impossible to correlate, and a compromised device can still expose several of them. They give us a boundary we can build on.

<details class="protocol-note" id="messaging-protocol-design">
<summary>A detour into messaging design: shared secrets and individual identity</summary>
<p>A shared secret and an individual signing key answer different questions. A message authenticated with a key everyone in the circle holds establishes that someone with that key made it. It cannot establish which member did so. A sender label does not change that: another holder of the same key could produce the same kind of proof.</p>
<p>A signing key gives a device something the other members do not have. It signs with its private key; they check the signature with its public key. Cahoots makes a separate signing key for each seat in each circle. The ceremony binds those public keys into a membership record that every seat signs. People still have to check the ceremony and know whose device is whose. A signature establishes control of a key, not a person's legal identity or good intentions.</p>
<p>This deliberately combines symmetric secrets for shared privacy with public-key cryptography for individual signatures. The exchange that establishes those secrets also uses public-key cryptography. The strongest one-time-pad claims apply only to messages encrypted with truly random secret bytes that are never reused. They do not extend to the signatures, the exchange, ordinary chat, or the whole system. Those still depend on conventional cryptographic assumptions.</p>
<p>There are other ways to design individual authentication, including schemes built from hashes. Choosing one would change the protocol and its tradeoffs; a shared group secret alone would still not distinguish the people holding it. The current design uses signing keys. A secure element can generate and use a private key without handing it to the laptop asking for a signature.</p>
<p>Hello words serve a different purpose again. They come from a secret shared by just two devices and let people check access to that secret during a call. They do not replace the signing keys used to approve actions. Keeping those jobs separate helps make each check's meaning clear.</p>
<p>The <a href="/docs/#/protocol">protocol specification</a> describes the mechanisms. The <a href="/docs/#/security">security guide</a> explains their limits. The goal here is to carry an established relationship between tools while being explicit about which keys protect shared information and which keys can authorize a particular action.</p>
</details>

Once the circle can recognize my decisions, I can also make a decision about who may help me. I should be able to authorize a particular job without giving a program my keys or all the permissions that come with them.

## Who gets to act for us

Suppose I ask a program to connect two of our work machines for the afternoon. It needs permission to establish that connection. It shouldn't also receive the keys to our conversations, the ability to open our files, or standing permission to authorize more work tomorrow.

The program can have its own key, and I can sign a grant naming that key and the job it's allowed to do. Then a service receiving its request can check both the program's signature and the authority I gave it. The program doesn't need to borrow my identity or receive my private key to carry out the task.

I want those grants to name the permitted operations, set a budget where one makes sense, and say when the authority ends. We should be able to read them and withdraw them when our plans change. If a program is allowed to pass work to another program, the authority it passes on should stay within the original grant. The tools carrying out the requests need to enforce those boundaries each time.

That gives us a way to decide which work can happen while we're busy and which actions still need a person to approve them. It follows from the same relationship we established at the table: we know whose key authorized the work, what they allowed, and where that permission stops.

For that to mean much, we need to trust the moment when we grant the authority. If a program controls the screen explaining its request, it can misrepresent what we're being asked to approve. If the computer holding our keys is compromised, it might approve the request without us at all. I want somewhere we can read the actual terms and make that decision ourselves. That's where I started wanting a separate piece of hardware.

## Somewhere to keep the keys

A phone can make and use a circle. It is also a general-purpose computer with a radio, a cloud account, and a few hundred other things going on. A browser is convenient, but the computer running it may be compromised. Putting keys there has consequences.

A badge gives us a chance to make those consequences smaller and more legible. A device with a narrow job. A screen you can read. Buttons you have to press. Keys that can stay in a secure element while the device uses them. Board files and firmware you can inspect, build, and argue with.

I want to plug it into a laptop and have it tell me what I'm approving. If I'm granting a program access, I want to see the scope and the limits on a screen that program doesn't control. I want to press the buttons myself. Then the program can get on with its work within those limits.

The phone and browser still belong in that everyday experience. They're where people already read, type, and talk. The badge should give the keys a physical home and give important decisions a place to happen. It shouldn't take a ceremony every time you want to have a conversation.

I want it to store the separate keys for several circles, too. The badge can recognize which circle a request belongs to and which key may answer it. My family, my friends, and the people I work with shouldn't become one public identity just because I carry one device.

Hardware wallets and login keys have done important work here. I want to bring that kind of deliberate custody to the private relationships between ordinary groups of people, and make it practical beyond a login screen or a transaction.

But now I've asked you to trust a device I'm building. If the answer stops there, I've handed you another box and another promise.

## The badge has to earn it, too

A screen is only useful if it shows enough to make a decision. A button is only useful if the firmware can't quietly go around it. An open board doesn't prove the device in your hand was built correctly.

Who built the binary? What dependencies went into it? Which firmware is the badge running? Did the factory deliver the chip its label claims? Otherwise we keep moving the demand for trust down a layer and calling the problem solved.

That's why I want verification to reach through the software stack and into the hardware supply chain. Publishing source lets people review what the software does. Reproducible builds let them compare the binary they rebuilt with the one distributed to users. Hardware needs its own evidence about what was manufactured and what's actually running.

Work like [Baochip-1x, the chip on the Dabao board](https://www.crowdsupply.com/baochip/dabao), matters to me because it pushes that question further, toward physical inspection of the silicon itself. The manufacturer's word shouldn't have to be our only evidence. That's the direction I want privacy hardware to go.

The same question applies to a hosted service. Where remote attestation is available, a client can check evidence of which program is handling its requests. That still leaves assumptions about the hardware and the machinery doing the checking. We should be able to explain what each check establishes and what it leaves in place.

Different circles will ask for different amounts of evidence. Some will use familiar phones. Some will want independently checked firmware. Some will want to inspect the hardware themselves. I want there to be a path for going deeper, all the way down to the silicon, with the remaining assumptions made explicit.

There are limits that inspection won't remove. The ceremony still relies on cryptography to keep an observer out of the exchange. Ordinary chat, file encryption, and network integrations use conventional cryptography and keep its limitations. A compromised member's device can expose what it holds. Another member can share a secret you wanted them to keep. A badge doesn't solve coercion. The [security guide](/docs/#/security) belongs alongside the things we hope to make possible.

Owning a tool ought to include knowing where it stops helping. It should also include being able to use it without spending your life checking it.

## On a day when you're tired

I don't want to become a part-time administrator just to have a private life. I don't think anyone else should have to, either.

A confusing ceremony can wear people down. So can a warning nobody understands or a stream of approval prompts. People start clicking through. They take shortcuts or leave. A beautiful security model becomes irrelevant.

Verification needs that same care. Ordinary users should be able to run the checks and understand what passed and what they still have to trust. They should be able to rely on an independent reviewer's work. Anyone who wants to inspect more deeply should be able to, but using the tool shouldn't require learning chip design or auditing every dependency.

I want to be able to answer a few questions without digging through a company's promises. Whose key am I trusting, and how did we agree it was theirs? Who else can read this? What would they have to do to read it? What happens if they're compromised? And when a program acts for us: who authorized it, what can it do, and how do we stop it?

Those answers need to live in the tools, where the people granting access can see and change them. Everyday use should make the safe path easy. We should save people's attention for decisions that deserve it.

That's the competency I care about: enough understanding, backed by usable tools, to have a say in your own affairs.

## What I want to keep

I started with choosing the right tools. I still care about that. But I want the relationships and work I build with those tools to have a life beyond any one of them. If a service changes tomorrow, I'd like a choice beyond accepting it or abandoning the people and work I've built there.

For a family, that might mean a private place to talk and keep things together. For a small organization, it might mean keys and connections it can keep using when a vendor has a bad week. It should be possible to let a machine help without giving it the run of the place.

That's what I mean when I ask what it would take for your digital footprint to be truly yours. Some of that footprint will always be out in the world, in other people's hands. I want us to have more say over the part we're still making: who we share it with, what we authorize, and what we can carry with us.

I'm building the badge and working out how to manufacture it because those choices need something behind them. They need keys we hold, identities grounded in our relationships, decisions we can enforce, and devices we have a chance of understanding. Privacy hardware needs a renaissance because more people deserve access to those things. There should be more boards, more approaches, and more tools we can own, fix, and inspect.

The next posts will get into the board, the constraints, the mistakes, and the work of connecting these pieces. If you've built hardware like this, or tried to, I'd like to hear what you ran into. The code and board files are on [GitHub](https://github.com/yourbuddyconner/cahoots). Build one. Tell me what's wrong with it. Help make the next one better.
