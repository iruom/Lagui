CXX = clang++
CXXFLAGS = -std=c++20 -O3 -DNDEBUG -Wall -Wextra -Wpedantic

.PHONY: bench bench-gui bench-cpp run run-basic run-metal test smoke smoke-basic smoke-metal
bench:
	cargo run --release --example draw_list_bench

bench-gui:
	cargo run --release --example gui_bench

run:
	cargo run --release --features portable --bin studio-demo

run-basic:
	cargo run --release --features portable --bin portable-demo

run-metal:
	cargo run --release --features native --bin native-demo

test:
	cargo test --features text

smoke:
	cargo run --release --features portable --bin studio-demo -- --smoke

smoke-basic:
	cargo run --release --features portable --bin portable-demo -- --smoke

smoke-metal:
	cargo run --release --features native --bin native-demo -- --smoke

bench-cpp: build/draw-list-bench
	./build/draw-list-bench

build/draw-list-bench: bench/draw_list.cpp include/bestgui/draw_list.hpp
	mkdir -p build
	$(CXX) $(CXXFLAGS) -Iinclude bench/draw_list.cpp -o $@
