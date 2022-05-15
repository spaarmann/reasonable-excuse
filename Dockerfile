FROM alpine:latest

COPY reasonable_excuse /app/reasonable_excuse
WORKDIR /app/
CMD [ "/app/reasonable_excuse" ]
