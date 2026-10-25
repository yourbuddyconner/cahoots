---
name: writing-blog-posts
description: Use when writing, editing, summarizing or excerpting a post for the Cahoots blog (blog/*.md), including the description line, the About box, figure captions, code-drawn cards, and tweet or thread versions of a post.
---

# Writing blog posts

## Overview

A Cahoots post is the author's argument in the author's words. The job is to carry the argument plainly, not to make it sound good. Every failure in this skill comes from one of two habits: summarizing the mechanism instead of the argument, and reaching for a phrase instead of saying the thing.

## The two rules

**1. A summary states the argument, not the mechanism.**
The description line, the About box, a tweet, a caption: each says what the post claims. "How Cahoots makes a secret" is the mechanism and belongs in the body.

| Mechanism (wrong) | Argument (right) |
|---|---|
| Every private tool depends on a secret an institution issues to you. Cahoots lets a group make that secret themselves when they meet in person. | The trust infrastructure underneath privacy was reliable for decades and is starting to fail. Privacy has to become a core competency for people and businesses, and that starts with hardware you can own. |

Test: can a reader who never learns how the ceremony works still tell what the post wants them to believe? If the summary only works once you know the mechanism, it is the mechanism.

The summary is third person unless the author asks otherwise. Third person means no "I"; it does not mean narrating the author ("The author spent years... and argues that..."). State the claim as a claim. Take its sentences from the post's own thesis lines; do not compose new ones.

**2. The author's sentences stay as written.**
Text the author pasted, dictated or edited is theirs. Edit only the sections you wrote. If one of their lines seems like a tell, leave it and say so in the reply.

## Voice

Plain explanation to a friend. Full sentences. One claim per sentence. The actor named. Said once.

What to strip from your own text, with the full list in `ai-patterns.md`:

- Reversals and mirrors. "The tunnel is yours. The introductions are theirs." "Not that it's broken. It isn't." "The keys of the people you do."
- Fragments standing in for sentences. "One-time pads. Couriers." "No server, no authority."
- Trailing qualifiers that carry the claim. "...with one more secret under it that no control plane ever saw."
- Objects as actors. "Cahoots is built so that..." Say who did it.
- Vague verbs. "introduces", "carries", "points to". Name the actual relation.
- Tailing negations. "Something you know how to do, not something you subscribe to."
- Filler. "actually", "simply", "precisely", "posture", "landscape".
- Nerd speak outside the spec. "make a shared secret together", "spend the pad", "pre-shared key". Say what it does for the reader. If a protocol word is unavoidable, use it once and explain it.

Before returning any text, run the compressed-claim tests from `ai-patterns.md` item 14 on each sentence you wrote.

## Shape of a post

Front matter: `title`, `date`, `author`, `role`, `tags`, `about`, `audience`, `learn` (a `- ` list), `description`, optional `image`, `slug`, `draft`. The build renders the About box, reading time, the "In this article" list from h2 headings, one OG card and one hero per post.

- Lead with the point. The first paragraph says what changed and what the author is doing about it. Do not bury the lede under context.
- Earnest over ideological. State what the author felt and what they decided. No manifestos of pure conviction with the facts withheld.
- Answer the obvious objection in its own section ("Why not just use Signal").
- One picture per idea that needs one. Real artifacts first (a screenshot, the repo's own diagram), then a code-drawn card using the `.cr` markup in `site/blog/blog.css`. Captions are one plain sentence.
- End by saying what comes next in the series, in prose, not by pointing at the site.

Build and check: `python3 scripts/build-blog.py --out site --skip-og`, then serve `site/`. Without `--skip-og` it also renders the cards with Chrome. The Pages workflow does the full render on push.

## Working with the author on copy

The author iterates by reaction. Each round, change what they pointed at and reread the rest for the same problem; do not recycle the previous draft with one line swapped. When they give a definition or a checklist, apply it to the whole post, not to the one sentence they quoted. When they say a line is wrong, do not propose a cleverer line. Find the sentence in the post that already says it.

## Red flags

- The summary mentions a room, a ceremony, randomness, or a secret being made. Mechanism.
- Two short sentences that mirror each other. Reversal.
- A sentence you are proud of. Rewrite it plainly.
- You changed a sentence the author wrote.
- The tweet thread introduces the product before the reason.
