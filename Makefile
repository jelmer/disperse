all: docker

docker: proto
	buildah build -t ghcr.io/jelmer/disperse:latest .
	buildah push ghcr.io/jelmer/disperse:latest
