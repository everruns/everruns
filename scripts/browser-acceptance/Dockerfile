# Disposable local acceptance browser. Not a production sandbox image.
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends chromium python3 ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 browser
USER browser
COPY run.py /opt/browser-acceptance.py
ENTRYPOINT ["python3", "-u", "/opt/browser-acceptance.py", "--worker"]
