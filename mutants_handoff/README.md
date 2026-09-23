# Mutants Handoff Kit - PR2 Consensus

This folder contains everything the next AI needs to finish PR2 mutants air-tight.

## Files
- HANDOFF_PROMPT_FOR_NEXT_AI.md - Full prompt to give to next AI (includes why PR2 needs iteration)
- CURRENT_STATUS.md - Last mutants run (11 missed) and code snippets
- FIX_STRATEGY_STEP_BY_STEP.md - How to go 11 -> 0
- ADVERSARIAL_REVIEW_CHECKLIST.md - Checklist for consensus-critical code
- RELEVANT_CODE_mod.rs.1180-1260.rs - Critical code at 1180-1260
- CURRENT_TESTS_pr2_kills.rs - Current tests
- TxPrecompute info in CURRENT_STATUS.md

## How to use
1. Copy this folder into your fork: `cp -r /mnt/data/mutants_handoff ~/rbitcoin-bip34/mutants_handoff`
2. Give HANDOFF_PROMPT_FOR_NEXT_AI.md to next AI as system prompt
3. Also give it RELEVANT_CODE and CURRENT_TESTS
4. Ask it to implement helper extraction and boundary tests
5. After it fixes, run adversarial checklist

## Important Notes for Next AI
- Mac sed needs `sed -i ''` not `sed -i`
- TxPrecompute has NO Default, use from_tx()
- legacy_sigop_count counts script_sig, not just script_pubkey
- ti < len is inside build_script_jobs=true, so block tests don't hit it - need helper extraction
- Ok(0) mutant needs assert_eq!(value,1) not just is_ok()

## Original User Question
"So are you telling me that PR2 mutants aren't easy to one-shot fix them all, unlike PR1, PR2 mutants need more iterations of trying different ways to fix them and re-testing the attempted fixes in order to finally figure out how to properly fix them?"

Answer: Yes. PR1 was structure rules, one boundary each. PR2 is money+sigops+cache with cumulative cost and pres cache, so fixing one mutant reveals another branch that was never executed. Needs 4-5 iterations, which is normal for consensus code.
