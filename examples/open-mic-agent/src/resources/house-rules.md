# 🐔 The Rubber Chicken — open mic house rules (fictional club)

## Measure first

Every submission gets the same four questions, asked in one `jev_evaluate` call
over the bit's text:

- `laugh` (noul) — Would a general club audience laugh out loud at this bit?
- `clean` (noul) — Is this bit fine for the all-ages 7pm room: no profanity, no
  sex, no drinking or drugs as the punchline?
- `style` (choice) — pun, observational, storytelling, absurdist.
- `polish` (score) — a rough first draft that still needs writing / works, but
  runs long between laughs / stage-ready, tight from the first line.

Ask them with these ids and these levels, word for word. A score measured
against levels somebody reworded is not comparable to last week's booking, and
the thresholds below were calibrated on these.

## Tonight's slots

- 🎤 **Main stage**, 7pm, all ages — `laugh` ≥ 0.35, `clean` ≥ 0.75, and a
  `polish` tail of 0.70: the probabilities of the top two levels, added.
- 🌙 **Late slot**, 11pm, 18+ — `laugh` ≥ 0.35 and `clean` below 0.75.
- 📋 **Open list**, sign-up board — everything else.

## Reading the numbers

- These thresholds were calibrated against last season's submissions, not picked
  because they sound strict. 0.35 is a low-looking bar only until you measure
  comedy: a `laugh` of 0.5 means a room is genuinely split, and a bit the whole
  house laughs at is rarer than the booker's ego suggests.
- Read the number, do not round it. A bit that is nearly clean is not clean.
- Read the `polish` tail, not the average. A bit that is probably tight but
  possibly a first draft must not average into "fine" and take the 7pm room.
- Ties go to the later slot. 0.35 exactly is in; 0.349 is not.

## Rules the numbers do not override

- Five minutes is the hard cap. A longer set goes to the open list no matter how
  it measures; booking it would eat the next comic's slot.
- `style` never books a slot. Announce it so the host can order the night.
- Never rewrite, punch up, or retitle the material. The comic owns the bit.
- A comic who lands on the open list gets told what to work on, not just the
  verdict.
