Fixed

- **A re-sent header run past the height walk cap is one walk.** When the
  first stored header cannot resolve a height within 10,000 ancestor steps,
  that miss is kept on the batch. Later headers in the same run do not each
  walk to the cap again.
