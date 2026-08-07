# Responsive knowledge-index list

## Description

Verify that populated knowledge-index records preserve readable identity, source, diagnostic state,
timestamps, and actions when the app content column narrows.

## Preconditions

- Everruns is running on the full DB-backed stack with authentication disabled.
- A knowledge index exists with a long name, description, ID, provider diagnostic, and every
  lifecycle action available.

## Test Data

| Field | Value |
|---|---|
| Route | `/knowledge-indexes` |
| Viewports | 320, 375, 390, 768, 1024, and 1440 pixels wide |

## Steps

1. Open `/knowledge-indexes` at each viewport.
2. Confirm the mobile/tablet record presents labeled Source, Index state, Last synced, Updated, and
   Actions fields instead of squeezing desktop columns.
3. Confirm the name, description, ID affordance, diagnostic text, and provider action remain inside
   the record.
4. Activate Open with pointer input. Focus Edit and Archive with the keyboard and confirm each is
   reachable without horizontal scrolling.
5. Confirm the page document does not overflow horizontally.
6. At desktop width, confirm the column headers and dense table presentation return.

## Expected Result

- Narrow records stack in content-priority order and every action remains visible and operable.
- Long text truncates or wraps inside the record without widening the page; the complete ID remains
  available through its copy affordance.
- Desktop retains the established table density and column headings.
