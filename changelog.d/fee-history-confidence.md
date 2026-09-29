Changed

- **Fee history conditions on windows that looked like now.** Historical fee estimates come from up to 1 GiB of recent transaction fee rows and use only past windows whose preceding blocks paid like the current ones, so an old fee spike no longer holds estimates high for months. Multi-block targets aim for 99% empirical inclusion and the 1-block target for 99.9%; blocks without fee-paying transactions do not count against them. Live flow data drives the near targets, blended with history, and a target without enough history has no historical rate.
