---
title: Privacy Hardware Needs a Renaissance
date: 2026-10-25
author: Conner Swann
role: Building Cahoots
tags: Building in public, Privacy
about: What could a small team establish for itself and keep as its tools change? Following that question through membership, individual decisions, and permission led me to hardware we can own and inspect.
audience: People who build and work in small groups, and the hardware and software builders who want to give them more control.
learn:
- Why a group should have a life beyond the services it uses
- What meeting in person can establish for our devices
- Why belonging to a group is different from approving a decision
- How a program could help without receiving everyone's authority
- Why important approvals led me to build a badge
- What ownership asks of the hardware, and of the people building it
description: A team already knows who belongs and who can make decisions. What would it take for its tools to build on those relationships? That question led me to Cahoots and hardware we can own and inspect.
---

Think of a small team building something together. They know one another, know who can make which decisions, and know when they need to ask. Their work lives across accounts, services, and machines. Each new tool brings another round of invitations and permissions, and another place where someone else maintains the record of who they are.

Over time, the team builds up more than a collection of files. It has a history, responsibilities, and ways of working together. But much of its ability to act together depends on accounts and settings it can't take with it when it leaves.

> What could that group establish for itself—and keep as its tools change?

I want to start with the people. They could agree on who belongs, give their devices a way to recognize one another, and decide which actions need whose approval. Tools could build on that foundation. The group would still need services, but it would hold some of what makes it a group in its own hands.

That's the idea behind Cahoots. Following it through the decisions a group needs to make is what led me to build a piece of hardware.

## A circle of our own

Imagine that team sitting down together and getting their devices out. As the devices exchange information, the people compare the words on their screens to check that everyone agrees. By the time they leave, their devices hold the keys and an agreed record of who belongs. We call that group a circle.

During this meeting, which we call a ceremony, each device contributes randomness that the devices combine into a shared secret. They also sign the membership record. There's no account provider choosing the members or certificate authority vouching for the people standing in front of you. The [ceremony guide](/docs/#/ceremony-guide) explains how that works and what everyone needs to check.

The point of meeting is what the group can do afterward. A tool that understands the circle could recognize its members from the record they agreed on together. The team would still decide what that tool may access and what each member may do. But the people and keys it starts with would come from the group itself.

### A check you already understand

This builds on an established pattern. Signal's [safety numbers](https://support.signal.org/hc/en-us/articles/360007060632-What-is-a-safety-number-and-why-do-I-see-that-it-changed) and Apple's [iMessage Contact Key Verification](https://security.apple.com/blog/imessage-contact-key-verification) already let people compare codes to check the keys behind a conversation.

When you sit down with someone you know to compare codes, you already know who you're looking at. The software doesn't. Checking the codes together gives you a way to connect the person you recognize, and the device they're holding, to the keys your software will use afterward.

I'd like that meeting to count beyond a conversation in one app. The team could bring its circle into other tools, with separate keys for each use. Chat, encrypted files, and private connections are early examples of where that foundation could help. Supporting another tool still takes an integration; creating a circle doesn't automatically make existing services recognize it.

<figure>
<img src="/blog/img/circle-and-tools.svg" alt="People meet in person to establish their circle. The circle supplies a common foundation for chat, files, and private connections, with separate keys for each use." width="640" height="480">
<figcaption>Early examples of the ambition: the group establishes its circle, and tools build on it with separate keys for each use.</figcaption>
</figure>

### Keeping the freedom to leave

I care about privacy. For a long time, that mostly meant choosing good tools and hoping the people behind them kept doing their jobs. I still care about those choices. But I want the relationships and work I build with those tools to have a life beyond any one of them.

The circle still needs infrastructure. We run a hosted relay so people can keep talking without first becoming server administrators, but delivering those messages doesn't require the relay to hold the circle's keys. Its operator can still observe traffic patterns or refuse delivery. Being able to run a different relay gives the group a choice about where its conversations travel.

That's one concrete piece of ownership: using a service while keeping enough in your hands to leave it. Carrying that freedom into more kinds of work is the ambition. It takes tools that honor the group's keys and decisions, as well as ways to move the work itself.

Cahoots is still a development project. Chat and the browser ceremony are available to try; integrations and hardware are at different stages. The release workflow below is an example of what I want to make possible, not a finished feature. The [status page](/docs/#/status) keeps track of that work.

## A place for each of us

Now suppose the team is getting ready to publish a release. Everyone may be allowed to read the project discussion, but the group still needs to know who approved the version going out. Knowing that someone belongs doesn't answer that question.

The shared secret helps protect what the group holds together. Every member has it, so a check made with that secret alone cannot tell us which member approved a decision. If three of us can make the same cryptographic proof, that proof cannot distinguish us.

The ceremony also gives each device its own signing key and records its public key in the circle's membership list. The private key stays with that device. When I sign something, the others can check it against the key we agreed belonged to my seat at the table. An application can then check whether my approval is the one it needs.

Trusting one another doesn't make those distinctions unnecessary. We can trust a colleague and still want to know which change they approved. We can agree that everyone belongs while leaving particular decisions to particular people.

### An identity for each relationship

I want that identity to live in the context of the relationship. My key in a work circle is separate from my key in a family circle. Neither needs to become a universal identifier that follows me everywhere.

A stolen key for one circle should not automatically give someone authority in another. Separate keys don't make the relationships impossible to correlate, and a compromised device can still expose several of them. They give us a boundary we can build on.

<details class="protocol-note" id="messaging-protocol-design">
<summary>A detour into messaging design: shared secrets and individual identity</summary>
<p>Alice and Bob should both be able to read our conversation. Only Alice should be able to approve an action as Alice. Those requirements need different evidence.</p>
<h3>Who can read, and who approved?</h3>
<p>A message authenticated with the group's shared key establishes that someone with that key made it. Every member can produce that kind of proof.</p>
<figure class="protocol-diagram" aria-labelledby="proof-caption">
<figcaption id="proof-caption">Shared access and individual approval need different evidence.</figcaption>
<div class="proof-grid">
<div class="proof-card">
<h4>One secret, held together</h4>
<div class="diagram-people">
<div class="diagram-person"><img class="cahoot-head" src="/blog/img/cahoot-alice.svg" alt="" width="48" height="48"><strong>Alice</strong><span class="key-label">group key</span></div>
<div class="diagram-person"><img class="cahoot-head" src="/blog/img/cahoot-bob.svg" alt="" width="48" height="48"><strong>Bob</strong><span class="key-label">group key</span></div>
</div>
<span class="diagram-arrow" aria-hidden="true">↓</span>
<div class="proof-result"><strong>A valid group-authenticated message</strong>Either member can create one.</div>
<p class="diagram-limit">The check establishes access to the group's key. It cannot tell Alice and Bob apart.</p>
</div>
<div class="proof-card">
<h4>A key that stays with Alice</h4>
<div class="diagram-person"><img class="cahoot-head" src="/blog/img/cahoot-alice.svg" alt="" width="48" height="48"><strong>Alice</strong><span class="key-label">Alice's private key</span></div>
<span class="diagram-arrow" aria-hidden="true">↓</span>
<div class="proof-result"><strong>Alice's signed approval</strong>Others check the signature with Alice's public key.</div>
<p class="diagram-limit">The private key is not shared. The check identifies which member key signed.</p>
</div>
</div>
</figure>
<p>During setup, Cahoots records a separate public signing key for each member in this circle. Shared secrets protect the group's communication. Individual signatures identify which member key signed a decision or a grant. An application can encrypt a signed decision to combine both checks.</p>
<h3>Give a helper permission without giving it your key</h3>
<p>Suppose Alice is allowed to authorize release preparation. She wants a helper to build a candidate for the team to review, while publication still needs a separate approval. A grant is her signed permission for the preparation job. The helper uses its own key.</p>
<figure class="protocol-diagram" aria-labelledby="grant-caption">
<figcaption id="grant-caption">A request needs both the right signer and permission for the job.</figcaption>
<ol class="grant-flow">
<li><div class="grant-step"><div class="grant-step-title"><img class="cahoot-head" src="/blog/img/cahoot-alice.svg" alt="" width="48" height="48">1. Alice signs a grant</div><span class="permission-slip">This helper may prepare a release for project A.<br>Permission ends at 6 pm. Publication needs separate approval.</span></div><span class="diagram-arrow" aria-hidden="true">↓</span></li>
<li><div class="grant-step"><div class="grant-step-title">2. The helper signs its request</div><p>It uses its own private key and presents Alice's grant. Alice keeps her private key.</p></div><span class="diagram-arrow" aria-hidden="true">↓</span></li>
<li><div class="grant-step"><div class="grant-step-title">3. The service checks both</div><p>Does the request come from the helper named in the grant? Was Alice allowed to authorize this job? Does her grant cover the requested operation, and is it still valid?</p></div></li>
</ol>
</figure>
<p>The service checks identity and permission separately: which key signed this request, and does Alice's grant let that key do this job? Access to the group's messages supplies neither approval.</p>
<p>Hello words check access to a secret shared by two devices during a call. They don't record permission to do a job. A signed grant can name the job, the resource, and the time limit.</p>
<p>Signatures establish control of keys. People still have to associate those keys with each other and protect them. Signatures, setup, and ordinary chat rely on conventional cryptographic assumptions. The strongest one-time-pad guarantees apply only to truly random secret bytes used once.</p>
<p>The <a href="/docs/#/protocol">protocol specification</a> describes the mechanisms. The <a href="/docs/#/security">security guide</a> explains what the checks establish and where their protection ends.</p>
</details>

## Who gets to act for us

Suppose the team wants a program to prepare its next release. It needs to read the project, run the build, and put the result somewhere the team can review it. Publishing that result is a separate decision. Permission to prepare a release shouldn't silently include permission to send it out.

The program can have its own key. An authorized member could sign a grant naming that key, the project, and the operations allowed for this job. A service receiving the program's request would check both its signature and the permission it presents. The program wouldn't need to borrow a member's identity or receive their private key.

When the build is ready, publication could require a fresh approval from the responsible member, tied to the particular result being released. The program could do the routine work while leaving that decision with a person.

### Permission with an end

I want those grants to say where the authority stops: which project, which operations, how much work where a budget makes sense, and when the permission ends. We should be able to read them and withdraw them when our plans change. If a program passes work to another program, the authority it passes on should stay within the original grant.

The tools carrying out requests have to enforce those rules. A signed permission slip doesn't constrain a service that ignores it. Supporting this release workflow would mean connecting the checks to the systems that build and publish the software.

For the team, the useful result would be ordinary: let a program get on with the job, know who authorized it, and keep the decisions it wasn't given. Those are the same responsibilities the people already understand, expressed in a form their tools can check.

### Somewhere to approve it

But what does the person see when it's time to approve publication? If the program asking for permission also controls the screen explaining the request, it can misrepresent what they're being asked to authorize. If the computer holding their keys is compromised, it might approve something without them at all.

I want a separate device that can show the actual request: this project, this release, this action. The person could check it and press a button to approve what they saw. That would require an approval tied to those details and a publishing tool that checks it before acting.

That's where I started wanting a badge.

## Somewhere to keep the keys

A phone can make and use a circle. It is also a general-purpose computer with a radio, a cloud account, and a few hundred other things going on. A browser is convenient, but the computer running it may be compromised. Putting keys there has consequences.

A badge gives us a chance to make those consequences smaller and more legible. I want it to have:

- A narrow job, with firmware we can inspect.
- A screen the program asking for permission doesn't control.
- Keys that can stay in a secure element, with physical buttons for granting authority.
- Board files we can inspect, build, and argue with.

For the team's release, I'd want the badge to display the project, the action, and an identifier for the exact build being approved. Its signature would cover those details. The publishing tool would check that signature and the signer's authority before it released anything. I'd need a trustworthy way to match that identifier to the build I'd reviewed.

The phone and browser still belong in that everyday experience. They're where people already read, type, and talk. The badge should give the keys a physical home and give important decisions a place to happen. It shouldn't take a ceremony every time you want to have a conversation.

### Several circles, one badge

I want it to store the separate keys for several circles, too. The badge can recognize which circle a request belongs to and which key may answer it. My family, my friends, and the people I work with shouldn't become one public identity just because I carry one device.

Hardware wallets and login keys have done important work here. I want to bring that kind of deliberate custody to the private relationships between ordinary groups of people, and make it practical beyond a login screen or a transaction.

But now I've asked you to trust a device I'm building. If the answer stops there, I've handed you another box and another promise.

## The badge has to earn it, too

A screen is only useful if it shows enough to make a decision. A button is only useful if the firmware can't quietly go around it. An open board doesn't prove the device in your hand was built correctly.

Who built the binary? What dependencies went into it? Which firmware is the badge running? Did the factory deliver the chip its label claims? Otherwise we keep moving the demand for trust down a layer and calling the problem solved.

<figure>
<img src="/blog/img/dependency.jpg" alt="A cartoon tower of blocks labeled all modern digital infrastructure, balanced on a few small pieces at the bottom" width="687" height="826" class="narrow">
<figcaption>The dependencies underneath our tools, after <a href="https://xkcd.com/2347/">xkcd 2347</a>. Giving a decision its own device still leaves us asking what that device depends on.</figcaption>
</figure>

### Evidence we can check

That's why I want verification to reach through the software stack and into the hardware supply chain. Publishing source lets people review what the software does. Reproducible builds let them compare the binary they rebuilt with the one distributed to users. Hardware needs its own evidence about what was manufactured and what's actually running.

Work like [Baochip-1x, the chip on the Dabao board](https://www.crowdsupply.com/baochip/dabao), matters to me because it pushes that question further, toward physical inspection of the silicon itself. The manufacturer's word shouldn't have to be our only evidence. That's the direction I want privacy hardware to go.

The same question applies to a hosted service. Where remote attestation is available, a client can check evidence of which program is handling its requests. That still leaves assumptions about the hardware and the machinery doing the checking. We should be able to explain what each check establishes and what it leaves in place.

Different circles will ask for different amounts of evidence. Some will use familiar phones. Some will want independently checked firmware. Some will want to inspect the hardware themselves. I want there to be a path for going deeper, all the way down to the silicon, with the remaining assumptions made explicit.

### The limits stay visible

There are limits that inspection won't remove. The ceremony still relies on cryptography to keep an observer out of the exchange. Ordinary chat, file encryption, and network integrations use conventional cryptography and keep its limitations. A compromised member's device can expose what it holds. Another member can share a secret you wanted them to keep. A badge doesn't solve coercion. The [security guide](/docs/#/security) belongs alongside the things we hope to make possible.

Owning a tool ought to include knowing where it stops helping. It should also include being able to use it without spending your life checking it.

## On a day when you're tired

I don't want to become a part-time administrator just to have a private life. I don't think anyone else should have to, either.

A confusing ceremony can wear people down. So can a warning nobody understands or a stream of approval prompts. People start clicking through. They take shortcuts or leave. A beautiful security model becomes irrelevant.

Verification needs that same care. Ordinary users should be able to run the checks and understand what passed and what they still have to trust. They should be able to rely on an independent reviewer's work. Anyone who wants to inspect more deeply should be able to, but using the tool shouldn't require learning chip design or auditing every dependency.

### Answers where the decisions happen

I want to be able to answer a few questions without digging through a company's promises:

- Whose key am I trusting, and how did we agree it was theirs?
- Who else can read this, and what would they need to do so?
- What happens if they're compromised?
- When a program acts for us, who authorized it, what can it do, and how do we stop it?

Those answers need to live in the tools, where the people granting access can see and change them. Everyday use should make the safe path easy. We should save people's attention for decisions that deserve it.

That's the competency I care about: enough understanding, backed by usable tools, to have a say in your own affairs.

## What I want to keep

Think back to the team at the table. Over time, its members will make decisions, bring in new tools, and ask programs to help. I want them to be able to trace those actions back to the relationships they established: who belongs, whose approval was needed, and what authority a helper received.

If they change services, I want them to keep enough of that foundation to continue their work. That takes more than keys. It takes integrations, ways to move their data, and services that honor their decisions. Holding the keys gives us somewhere to begin building those choices.

A family or another small group with lasting responsibilities could need the same things. The decisions would be different, but belonging, individual approval, and permission to help would still matter. Trusting someone shouldn't require handing them every key or making every permission permanent.

That's the part of our digital lives I want us to have more say over: what we establish together, what we authorize, and what we can carry with us.

I'm building the badge and working out how to manufacture it because those choices need something behind them. They need keys we hold, identities grounded in our relationships, decisions our tools can enforce, and devices we have a chance of understanding.

Privacy hardware needs a renaissance because more people deserve access to those things. There should be more boards, more approaches, and more tools we can own, fix, and inspect.

The next posts will get into the board, the constraints, the mistakes, and the work of connecting these pieces. If you've built hardware like this, or tried to, I'd like to hear what you ran into.

The code and board files are on [GitHub](https://github.com/yourbuddyconner/cahoots). Build one. Tell me what's wrong with it. Help make the next one better.
