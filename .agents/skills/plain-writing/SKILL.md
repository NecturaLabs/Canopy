---
name: plain-writing
description: How to write Canopy's words for people and agents — pull request bodies, reviews and comments, issue text, commit subjects, UI copy, docs and the changelog — plainly, verdict first, with no filler, praise or restatement. Use whenever writing or editing any of these. Before a pull request is opened and before it is merged, the canopy-slop-check agent checks the result against this skill.
---

# Plain writing

Every line has to earn its place. Write the outcome first, then only what the reader needs to act.

## Pull request bodies, reviews and comments

Two readers, in this order: a human reviewer who reads the top and stops, then an engineering
agent that reads to the end and acts on it.

Mark the two parts with headings, `### For human reviewers` and `### For agent reviewers`, so each
reader knows where to stop or start.

1. **Top, for the human:** the verdict or outcome in one sentence (approve, changes needed, what
   this does), then one line per blocking item. A human who stops here knows what happens next.
2. **Below, for the agent:** per item, the requirement, where it applies (file, symbol, platform),
   the evidence (command, commit, observed output) and a checkable done condition. Plain
   imperative sentences; detail only where an implementer needs it.
3. Optional and decided items last, one line each, in the human part.

One comment per round. A correction or addition edits the review or comment already posted;
never post a second one.

Avoid: an opening that thanks, praises or summarises before the verdict; a blocking item that
needs a paragraph to be understood; detail the top already said; a requirement scoped narrower
or wider than the issue or the owner's word (for example a platform named where the requirement
holds on all of them); a claim not backed by what was run.

## The test for every line

1. **Is it already said or shown?** A restatement goes.
2. **Would the reader miss it?** If nothing goes wrong without it, delete it. Deleting beats rewording.
3. **Is it the plain way to say it?** What happened, what to do, in the project's own words.

## Signs to look for

From [Wikipedia's Signs of AI writing](https://en.wikipedia.org/wiki/Wikipedia:Signs_of_AI_writing):

- **Inflated importance:** crucial, pivotal, vital, key, robust, seamless, comprehensive, leverage,
  delve, landscape, showcase, underscores, highlights, ensures.
- **Avoiding "is" and "has":** serves as, stands as, features, boasts, offers.
- **Negative parallels:** "not just X, but Y", "not X, it's Y".
- **Trailing clauses that add a claim:** ", ensuring…", ", making it…", ", which means…".
- **Reflex threes** and lists padded to look complete.
- **Hedges and fillers:** please note, it's worth noting, simply, just, great question, happy to.
- **Praise and thanks as padding:** "Thanks for this", "solid work", "saved us a lot of digging",
  "great catch". One short thanks at the very end is allowed when the brief asks for warmth.
- **Chat leftovers:** "Certainly", "I hope this helps", "let me know if", emoji, stray `**`,
  "🤖 Generated with…" footers.
- **Two sentences where one says it**, and explaining the mechanism when the reader needs only
  the outcome.

UI copy also follows the owner's rule: one plain fact per line, no welcome text, no tooltip that
repeats its label.

## What is fine

- **Signs alone.** One word in a plain, useful sentence is fine. Judge the line, never its origin;
  never call text AI-written and never suggest evading a detector.
- **Facts that change a decision:** what is lost, that it cannot be undone, what was not verified,
  how to recover. Keep the fact and trim the sentence.
- **Git and product terms**, command names, file paths, error text quoted from git.
