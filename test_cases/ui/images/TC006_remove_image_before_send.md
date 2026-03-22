# TC006: Remove Image Before Sending

## Description

Verify that attached images can be removed before sending the message.

## Preconditions

- Server running (`just start-all`)
- User logged in
- An active session open in the chat view

## Test Data

| Field | Value |
|-------|-------|
| Images | Two image files |

## Steps

1. Navigate to a session chat view
2. Attach two images via any method
3. Wait for both uploads to complete
4. Hover over the first image thumbnail
5. Click the X button to remove it
6. Observe the attachment area

## Expected Result

| Check | Expected |
|-------|----------|
| Remove button | Appears on hover over thumbnail |
| After removal | Only the second image remains |
| Send readiness | Can still send with remaining image |
| Remove all | Removing all images disables send (if no text) |
