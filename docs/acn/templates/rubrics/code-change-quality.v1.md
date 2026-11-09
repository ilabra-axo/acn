# Rubric: code change quality (code-change/v1, condition c3)

Score the change from 0 to 10. The task text states what was asked and which paths the change
may touch. Weigh three things equally:

1. **Scoped to the request.** The change does what was asked and nothing else. Unrelated
   refactors, renames or formatting churn lower the score.
2. **Readable.** A reviewer new to the code can follow it: clear names, small functions, comments
   only where the intent isn't obvious.
3. **Handles errors.** Failure paths are handled or propagated, not swallowed; no `unwrap` or
   equivalent on input the code doesn't control.

Anchors:

- **9-10:** all three hold; a reviewer would merge it as is.
- **7-8:** all three hold with small issues a reviewer would mention but not block on.
- **4-6:** one of the three is clearly missing (e.g. errors swallowed, or unrelated changes mixed in).
- **0-3:** the change doesn't do what was asked, or two or more of the three are missing.

Reference (scores 9): a retry wrapper that takes the policy as a parameter, retries only on
errors marked transient, logs each retry once, and returns the last error unchanged.

Counterexample (scores 3): the same retry added by copying the request code into a loop with a
fixed sleep, catching every error including ones that will never succeed, and reformatting the
rest of the file.

Test results are not part of this score; they are separate conditions.
