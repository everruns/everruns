# TC002: Drag and Drop Image

## Description

Verify that dragging and dropping an image file onto the chat input uploads it successfully.

## Preconditions

- Server running (`just start-all`)
- User logged in
- An active session open in the chat view
- An image file accessible in the file system

## Test Data

| Field | Value |
|-------|-------|
| Image file | Any PNG, JPEG, GIF, or WebP file under 100 MB |

## Steps

1. Navigate to a session chat view
2. Open a file manager window alongside the browser
3. Drag an image file from the file manager onto the message input area
4. Observe the drop zone highlight and image attachment

## Expected Result

| Check | Expected |
|-------|----------|
| Drop zone highlight | Input area shows visual highlight during drag-over |
| Image thumbnail | Appears in attachment area after drop |
| Upload completes | Spinner replaced by image preview |
| Filename shown | Shows original filename below thumbnail |
