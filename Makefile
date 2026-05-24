.PHONY: all build clean

all: build

build:
	@echo "Building Docker image..."
	docker build -t nanos-builder .
	@echo "Running build process..."
	mkdir -p output
	docker run --rm --privileged -v "$(PWD)/config:/build/config" -v "$(PWD)/output:/build/output" nanos-builder

clean:
	@echo "Cleaning output directory..."
	rm -rf output/
