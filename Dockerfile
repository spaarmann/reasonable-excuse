FROM alpine:latest

# To use, mount a valid config to `/app/config.kdl`
COPY build/reasonable-excuse /app/reasonable-excuse
WORKDIR /app/
CMD [ "/app/reasonable-excuse" ]
