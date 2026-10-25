# AI writing patterns

Condensed from Wikipedia's "Signs of AI writing" (WikiProject AI Cleanup). For each pattern: what to watch for, and the fix. Look for clusters. One tell means little, several together are a confession.

## Content

1. Inflated significance. `stands as a testament`, `pivotal moment`, `underscores its importance`, `reflects broader`, `marking a shift`, `evolving landscape`, `indelible mark`. Cut the ceremony; state the fact.
2. Notability name-dropping. Lists of outlets or follower counts without context. Keep the one or two citations that carry real context; drop the rest.
3. Superficial `-ing` tack-ons. `...highlighting`, `...ensuring`, `...showcasing`, `...fostering`. Delete the participle clause or convert it to a concrete claim.
4. Promotional language. `boasts`, `vibrant`, `rich` (figurative), `nestled`, `renowned`, `stunning`, `commitment to`. Replace with neutral description.
5. Vague attributions. `Experts argue`, `Industry reports`, `Observers have cited`. Name the source or cut the claim. Never invent a source.
6. Formulaic "Challenges" sections. `Despite its X, faces several challenges... continues to thrive`. Keep only the concrete facts; cut the frame.

## Language

7. AI-frequency vocabulary. `delve`, `tapestry`, `testament`, `pivotal`, `crucial`, `vibrant`, `landscape` (abstract), `interplay`, `intricate`, `showcase`, `underscore`, `foster`, `garner`, `enduring`, `enhance`, `align with`, `key` (adjective), `posture`. Swap for plain words.
8. Copula avoidance. `serves as`, `stands as`, `boasts`, `features`, `offers` where `is`/`has` works. Use the copula.
9. Negative parallelism and tailing negations. `Not only X but Y`, `It's not just about X, it's Y`, `X, not Y`. State the claim directly.
10. Rule of three. Forced triads. Keep the items that are real; break the rhythm.
11. Synonym cycling. One name for one thing.
12. False ranges. `from X to Y` where X and Y aren't on a scale. List the items plainly.
13. Subjectless fragments. `No configuration file needed.` Restore the actor: `You do not need a configuration file.`
14. Compressed-claim sentences. One claim per sentence. The cadence tell is list, verb, trailing qualifier, full stop.
    - Shape one, vague reference verbs. `the blocker Zeke named` → `the blocker Zeke raised`. Also `points to`, `carries`, `captures`, `produced`, `introduces`. Test: if the verb could describe any relation between the two nouns, replace it with the specific relation.
    - Shape two, the claim sits in a trailing qualifier. `A reviewer can verify the record later without the harness that produced it.` → `Verification does not require the harness. Anyone with the record and the public keys can check it.` Test: for each sentence that ends in a qualifier after a complete clause, ask whether the qualifier is the main point. If it is, give it its own sentence. Emphatic tails like `and nothing else` are the same shape.
    - Shape three, the object as actor. `A change can merge when a valid record exists.` → `The gate checks the signatures. A reviewer can merge only when the check passes.` Test: name who performs the action, and replace adjectives like `valid` and intensifiers like `exact` with the conditions they stand for.

## Style

15. Em and en dashes. Zero in the final text. Replace with period, comma, colon, or parentheses.
16. Mechanical boldface. Unbold unless emphasis is load-bearing.
17. Inline-header lists. `- **Label:** sentence restating the label`. Collapse to prose or a plain list.
18. Title Case headings. Sentence-case them.
19. Emoji decoration. Remove.
20. Curly quotes. The renderer curls them; do not type them.

## Communication

21. Chat artifacts. `I hope this helps!`, `Let me know...`. Delete.
22. Cutoff disclaimers and gap-filling. Say what isn't known or cut it; never dress a guess as fact.
23. Sycophancy. Delete.

## Filler and rhythm

24. Filler phrases. `in order to` → `to`, `due to the fact that` → `because`, `it is important to note that` → cut. Also `actually`, `simply`, `precisely` when they add nothing.
25. Hedging stacks. One hedge maximum, and only if the uncertainty is real.
26. Generic upbeat conclusions. End on the last concrete fact.
27. Uniform hyphenation. Keep attributive hyphens; drop predicate ones.
28. Authority tropes. `The real question is`, `at its core`, `what really matters`. State the point.
29. Signposting. `Let's dive in`, `Look at where...`. Do the thing instead of announcing it.
30. Fragmented headers. A heading followed by a one-liner restating it. Cut the restatement.
31. Diff-anchored writing. Describe the current state; save change narration for changelogs.
32. Staccato drama. Runs of short fragments (`No aesthetic prior. No nostalgia.`). One short sentence for emphasis is fine; a run is engineered.
33. Aphorism formulas and reversals. `X is the Y of Z`, `The tunnel is yours. The introductions are theirs.`, `If X is wrong you lose nothing; if X is right you were already there.` Replace with the concrete claim.
34. Fake-candid openers. `Honestly?`, `Here's the thing`. Just say the thing.

## Do not flag

Polished grammar, formal vocabulary in general, a single short emphatic sentence, one `however`, quoted or secondhand text, and anything the author wrote themselves.

## Preserve

Specific hard-to-fabricate detail, mixed feelings, era-bound references, first-person editorial choices, varied sentence length, genuine asides and self-corrections. Edit lightly; over-editing destroys what makes the piece human.
