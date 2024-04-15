FROM rust:latest

RUN apt update
RUN apt install -y build-essential
RUN apt install -y python3-pip

COPY ./ ./

CMD ["cargo", "build", "--release"]
