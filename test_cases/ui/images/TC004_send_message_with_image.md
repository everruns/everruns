# TC004: Send Message with Image Attachment

## Description

Verify that a message with attached images is sent successfully and displayed in the chat.

## Preconditions

- Server running (`just start-all`)
- User logged in
- An active session open in the chat view

## Test Data

| Field | Value |
|-------|-------|
| Message text | "Describe this image" |
| Image | Any PNG or JPEG file |

## Steps

1. Navigate to a session chat view
2. Attach an image via paste, drag-drop, or file picker
3. Wait for the upload to complete (spinner disappears)
4. Type "Describe this image" in the message input
5. Press Enter or click the Send button
6. Observe the sent message in the chat

## Expected Result

| Check | Expected |
|-------|----------|
| Send button | Enabled only after image upload completes |
| Sent message | Shows text and image thumbnail in chat history |
| Image clickable | Clicking thumbnail opens full-size preview dialog |
| Download button | Preview dialog has a download button |
| Agent response | Agent processes the image (no deserialization error) |
