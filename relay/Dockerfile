# Build stage
FROM golang:1.25-alpine AS builder

WORKDIR /app

# Copy go.mod and go.sum first for better caching
COPY go.mod go.sum ./
RUN go mod download

# Copy the rest of the source code
COPY . .

# Build the application
RUN CGO_ENABLED=0 GOOS=linux go build -o cdus-relay ./cmd/cdus-relay

# Final stage
FROM alpine:latest

WORKDIR /app

# Copy the binary and static files from the builder stage
COPY --from=builder /app/cdus-relay .
COPY --from=builder /app/static ./static

# Create a directory for the database
RUN mkdir -p /data

# Set environment variables
ENV PORT=7860
ENV DB_PATH=/data/relay.db

# Expose the port used by Hugging Face Spaces
EXPOSE 7860

# Run the application
CMD ["./cdus-relay", "-port", "7860", "-db", "/data/relay.db"]
