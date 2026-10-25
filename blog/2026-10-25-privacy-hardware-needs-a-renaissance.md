---
title: Privacy Hardware Needs a Renaissance
date: 2026-10-25
author: Conner Swann
role: Building Cahoots
tags: Building in public, Privacy
about: Why Cahoots exists. A wake-up call about the trust infrastructure underneath privacy, what the real dependency is, and what a group can do about it on its own.
audience: Anyone who cares about privacy or digital sovereignty, and teams that would rather run their own infrastructure than rent it.
learn:
- Why the certificate system failing is a canary, not the whole problem
- What the real dependency is: the first secret, not the encryption
- Which tools you can already run yourself once you have shared randomness
- What Cahoots does, and which parts of your life it is for
- Why privacy has to become a core competency for people and businesses
description: The certificate system is just the canary. The tools for private infrastructure already exist. What is missing is the first secret, made by the people who will use it and nobody else.
---

I care about privacy. I have for a long time. And for most of that time I could care about it comfortably, because the infrastructure underneath it was somebody else's problem. Certificate authorities, roots of trust, the quiet machinery that decides which connections are real. It had been boring and reliable for decades, not worth losing sleep over. You never thought about it because you never had to.

The last couple of weeks, that stopped being true for me.

I'm not going to relitigate what happened. You've seen it. What matters is what it revealed: the layer I'd treated as bedrock is people and scripts like everything else. It can be wrong for a year and a half before anyone looks. And when it is wrong, I have no say in how it gets fixed, no way to see it coming, and no alternative to fall back on.

That was my wake-up call. Not that everything is broken. It isn't. But I had built my entire sense of privacy on a foundation I had never examined and could not repair. I was a tenant in a rent-seeking economy, and I'd never met the landlord.

## The canary

The certificate system is just the canary. The same dependence runs through everything.

Your VPN trusts a company. Your messaging app trusts a company. Your login trusts a company, which trusts another company, which trusts a certificate authority. Every private thing in your life runs through a chain of institutions you did not choose, cannot inspect, and cannot replace. Most of the time they do their jobs. When they don't, you find out after the fact, if you find out at all.

I don't think those institutions are villains. I think they're ordinary people like you and me, and ordinary things fail. The question is whether you have anything that is truly yours when they do.

## The tools already exist

Here's what I keep coming back to. The technology for private infrastructure is already built, and setting aside any spectres of the quantum singularity they are already open, and already good. VPNs. Mutual TLS. Plain old private networking. Encrypted files. Group chat. None of it is exotic. You can run all of it yourself, on your own machines, for your own people.

Before any of those tools can protect anything, somebody has to hand out the first secret, and it has to reach the right people and nobody else. Today that somebody is an institution. You apply, it issues, you trust. The dependency was never in the math. It's in the order of operations.

If you have a source of randomness that you share with the people you trust, and nobody else, you can build your own internet infrastructure on top of it. Your own tunnels. Your own encrypted connections. Your own signing. Your own small piece of the internet, with nothing in the middle.

## What Cahoots is

Cahoots is a way to get that first secret without needing to rent it from someone who doesn't care about you outside of whatever immediate value you can provide them.

A group of people meets in person. Friends, a family, a team, a company. Each person's device contributes randomness, and everyone leaves with the same shared secret, which no server, no authority, and no third party ever touched. From there, every key you need to communicate with that group comes out of it. A key for a VPN. A key for a connection between two of your machines. A key for a file, or a chat, or a message that nothing can read.

That covers a lot of what people and small organizations actually need from each other. It doesn't cover the public web, and it isn't meant to. Browsing a stranger's website will always need a system that scales to strangers. Cahoots is for the other side of your life: the people you already know, the machines you already run, the things that are actually yours.

## Privacy as a core competency

I want to address the elephant in the room directly. Privacy has to become a core competency, for individuals and for businesses. Something you know how to do, the way you know how to lock your door, rather than something you buy once and forget about.

For a person, that means being able to keep things between you and the people you choose, with no company in the loop. For a business, it means running your own connections, your own network, your own trust, so that a stranger's mistake never becomes your outage or your breach.

The tools are there. The knowledge is learnable. What's been missing is the first step, and the habit.

## What comes next

This post is the why. The rest of this blog is the how, and the how is mostly hardware.

A phone can run Cahoots today, and the browser app proves the protocol works. But a phone is a general-purpose computer with a radio, a cloud account, and a few hundred apps that would love to know what it's doing. Privacy hardware has a different job: do one thing, hold one secret, and give an attacker as little surface as possible. That means a device with no network, a screen you can read, a button you can press, and a way to hand randomness to the device next to it with nothing in between. It also means parts you can buy, a board you can inspect, and a design you can build yourself.

That is a harder problem than it sounds, and it has had very little attention outside of wallets and login keys. Those got us partway. I think it's time for another run at it.

So the next post starts the series on getting the badge manufactured. It covers the design of the first board and the constraints that shaped it: what privacy hardware has to do differently from everything else on your desk, which of those constraints are physics and which are habit, and the tradeoffs I made for a first run.

If you've built hardware like this, or tried to, I'd like to hear what you ran into. The code and the board files are on [GitHub](https://github.com/yourbuddyconner/cahoots), and the mistakes will be here.
