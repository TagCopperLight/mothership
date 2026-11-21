FROM scratch
COPY app /app
HEALTHCHECK --interval=2s --timeout=2s --retries=2 CMD ["/app", "health"]
ENTRYPOINT ["/app"]
