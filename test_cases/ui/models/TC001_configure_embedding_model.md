# TC001 Configure an embedding model

## Description

Verify that an embedding model can be configured and is offered to knowledge indexes without
appearing in chat-model selectors.

## Preconditions

- A local (`AUTH_MODE=none`) or authenticated Everruns UI is available
- An OpenAI provider is configured
- No enabled model has the `embeddings` capability

## Test Data

| Field | Value |
| --- | --- |
| Provider | Any configured OpenAI provider |
| Model type | Embeddings |
| Model ID | `text-embedding-3-small` |
| Display name | Text Embedding 3 Small |

## Steps

1. Navigate to `/knowledge-indexes` and open the New Knowledge Index dialog.
2. Verify the embedding-model field reports that no embedding models are configured and offers a
   Configure an embedding model link.
3. Follow the link to `/models`, click Add Model, and select the OpenAI provider.
4. Select Embeddings as the model type, enter the model ID and display name, leave the model
   enabled, and create it.
5. Return to `/knowledge-indexes`, reopen the New Knowledge Index dialog, and inspect the
   embedding-model options.
6. Inspect an agent, harness, or organization default-model selector.

## Expected Result

- The empty embedding picker links to model configuration.
- The model is created with the embeddings capability and appears in the knowledge-index picker.
- Chat models do not appear in the embedding picker.
- The embedding model does not appear in chat or default-model selectors.
