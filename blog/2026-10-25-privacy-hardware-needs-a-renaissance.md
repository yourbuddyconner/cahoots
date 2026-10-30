---
title: Privacy Hardware Needs a Renaissance
date: 2026-10-25
author: Conner Swann
role: Building Cahoots
tags: Building in public, Privacy
about: What would it take for your digital footprint to be truly yours? Why that question led me to circles, deliberate authority, and hardware you can own and inspect.
audience: People who want more control over their private lives, and the hardware and software builders who want to help.
learn:
- Why choosing good privacy tools still leaves me wanting more
- How a group could carry its trust between tools
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

There are good tools here. I use Signal and Tailscale. A password manager and a hardware security key are good things to have. These tools give people real protections, including ways to verify each other and keep keys on their own devices.

Putting them together still asks a lot of the person using them. Each has its own way to establish trust, its own recovery story, and its own boundaries. The relationship you verified in one place doesn't automatically help you in the next. The group keeps starting over.

I'd like us to have something durable underneath those tools. Something we establish ourselves and can keep using as the tools change.

The people you'd hand a house key to, you have already met. That should count for more.

## Something the group can keep

Imagine meeting up with a few people you trust. You get your devices out, exchange some information, and compare the words on your screens. Everyone checks that they match. You leave with a private circle you made together.

Later, you open that circle and talk. You protect a file for the group. You connect two machines that belong to its members. You can see who belongs, and you have keys for doing things together. The next private thing starts with a relationship you've already established.

That's the starting point for Cahoots. The meeting is called a ceremony. Each device contributes randomness, and the devices combine those contributions into a shared secret. They also sign a record of who belongs. The [ceremony guide](/docs/#/ceremony-guide) explains what everyone needs to check.

No account provider chooses the members. No certificate authority needs to vouch for the people standing in front of you. No relay gets a copy of the circle's secret.

<figure>
<img src="/blog/img/circle-and-tools.svg" alt="People meet in person to establish their circle. The circle supplies a common foundation for chat, files, and private connections, with separate keys for each use." width="640" height="480">
<figcaption>The ambition: establish the circle together, then carry it into the tools you use. Each use gets its own keys.</figcaption>
</figure>

There will still be infrastructure. Messages need delivery. Software needs updates. Devices break. We run a hosted relay because people should be able to start a conversation without first becoming server administrators. The relay doesn't need the circle's keys to deliver its messages. It can still observe traffic patterns or refuse delivery. You can run your own if you want that decision in your hands.

That's a boundary I want to make clear: here's what the service does, here's what it can learn, and here's what remains with us. Moving the conversation to another relay should be a choice the group can make.

Cahoots is a development project. Chat and the browser ceremony are things you can try; the integrations and hardware are at different stages. The [status page](/docs/#/status) keeps track of that. What I'm describing here is the direction: making the circle once should make the next private thing easier.

But as soon as a circle becomes useful, another question follows. What happens when we want a program to do some of that work for us?

## Who gets to act for us

An agent arranging a private connection for one job shouldn't need every secret the circle holds. It needs enough authority to do that job, for long enough to finish it. We should be able to tell what we gave it and take that authority back.

Knowing who signed a request doesn't tell you what they're allowed to do. A program acting for me needs its own identity and a clear grant of authority. Handing it my account and hoping it behaves is a miserable foundation for that future.

I want the circle's members to decide what they grant. Which operations can the program perform? What can it spend? When does its authority end? Can it delegate any of the work? If it does, the next program should receive only a narrower portion of the authority it was given. Services should refuse requests outside those permissions.

Some actions should always need a person to approve them. Others should be able to happen while we're busy with something else. Having agency shouldn't mean pressing a button for every routine action. It should mean having a real say in which actions can happen without us.

That makes the moment of approval matter a great deal. If a program asks for permission on a screen it controls, what exactly am I agreeing to? If the computer holding my keys is compromised, what stops it from agreeing for me?

The circle needs somewhere we can make those decisions deliberately. That's where I started wanting a separate piece of hardware.

## Somewhere to keep the keys

A phone can make and use a circle. It is also a general-purpose computer with a radio, a cloud account, and a few hundred other things going on. A browser is convenient, but the computer running it may be compromised. Putting keys there has consequences.

A badge gives us a chance to make those consequences smaller and more legible. A device with a narrow job. A screen you can read. Buttons you have to press. Keys that can stay in a secure element while the device uses them. Board files and firmware you can inspect, build, and argue with.

I want to plug it into a laptop and have it tell me what I'm approving. If I'm granting a program access, I want to see the scope and the limits on a screen that program doesn't control. I want to press the buttons myself. Then the program can get on with its work within those limits.

The phone and browser still belong in that everyday experience. They're where people already read, type, and talk. The badge should give the keys a physical home and give important decisions a place to happen. It shouldn't take a ceremony every time you want to have a conversation.

I want it to store keys for several circles, too. My family, my friends, and the people I work with shouldn't become one public identity just because I carry one device.

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

I want to be able to answer a few questions without digging through a company's promises. Who else can read this? What would they have to do to read it? What happens if they're compromised? And when a program acts for us: who authorized it, what can it do, and how do we stop it?

Those answers need to live in the tools, where the people granting access can see and change them. Everyday use should make the safe path easy. We should save people's attention for decisions that deserve it.

That's the competency I care about: enough understanding, backed by usable tools, to have a say in your own affairs.

## What I want to keep

I started with choosing the right tools. I still care about that. But I want the relationships and work I build with those tools to have a life beyond any one of them. If a service changes tomorrow, I'd like a choice beyond accepting it or abandoning the people and work I've built there.

For a family, that might mean a private place to talk and keep things together. For a small organization, it might mean keys and connections it can keep using when a vendor has a bad week. It should be possible to let a machine help without giving it the run of the place.

That's what I mean when I ask what it would take for your digital footprint to be truly yours. Some of that footprint will always be out in the world, in other people's hands. I want us to have more say over the part we're still making: who we share it with, what we authorize, and what we can carry with us.

I'm building the badge and working out how to manufacture it because those choices need something behind them. They need keys we hold, decisions we can enforce, and devices we have a chance of understanding. Privacy hardware needs a renaissance because more people deserve access to those things. There should be more boards, more approaches, and more tools we can own, fix, and inspect.

The next posts will get into the board, the constraints, the mistakes, and the work of connecting these pieces. If you've built hardware like this, or tried to, I'd like to hear what you ran into. The code and board files are on [GitHub](https://github.com/yourbuddyconner/cahoots). Build one. Tell me what's wrong with it. Help make the next one better.
