# TC001: Paste Image via Ctrl+V

## Description

Verify that pasting an image from clipboard into the chat input uploads it successfully.

## Preconditions

- Server running (`just start-all`)
- User logged in
- An active session open in the chat view
- An image copied to the clipboard (e.g. screenshot or copied from another app)

## Test Data

| Field | Value |
|-------|-------|
| Image source | Screenshot or copied PNG/JPEG from clipboard |

## Steps

1. Navigate to a session chat view
2. Click into the message input textarea
3. Press Ctrl+V (or Cmd+V on macOS) with an image on the clipboard
4. Observe the image attachment area above the input

## Expected Result

| Check | Expected |
|-------|----------|
| Image thumbnail | Appears in attachment area with loading spinner |
| Upload completes | Spinner replaced by image preview (no error icon) |
| Filename shown | Shows generated filename (e.g. "image.png") below thumbnail |
| No error | No "Failed to deserialize" or "API Error" message |
| Remove button | Hover over thumbnail shows X button to remove |
