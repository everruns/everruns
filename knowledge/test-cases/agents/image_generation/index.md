# Image generation (Agent workflow)

* [TC001: GPT Image Generation with Session Files and Persisted Artifacts](TC001_funny_images_with_artifacts.md) - Verify that an agent using `image_generation` can complete a three-image generation request end to end: plan three `generate_image` calls, persist durable `img_*` artifacts, mirror each image into the session filesystem,...
* [TC002: Animated GIF on the Organization Default Image Model](TC002_animated_gif_in_chat.md) - Verify that a Worker-kit agent with no image settings draws an animated GIF on the organization's default image model, stores it as an image/gif artifact, and that chat plays it with a frame badge, pause and download.
