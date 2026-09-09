#include <bestgui/draw_list.hpp>
#include <algorithm>
#include <array>
#include <chrono>
#include <cstdio>
#include <vector>

using Clock = std::chrono::steady_clock;

// Compiler barrier: construction must be observable before stopping the timer.
static void observe(const bestgui::Quad* data) {
#if defined(__clang__) || defined(__GNUC__)
    asm volatile("" : : "r"(data) : "memory");
#else
#error This benchmark requires a compiler memory barrier implementation.
#endif
}

int main() {
    std::array<bestgui::Quad, 4096> storage{};
    bestgui::DrawList list(storage);
    // Exercise capacity, order, and reset with checks active in release builds.
    for (std::size_t i = 0; i < storage.size(); ++i)
        if (!list.rectangle({float(i), 0, 10, 10}, {1, 1, 1, 1})) return 1;
    if (list.rectangle({}, {}) || !list.overflowed()) return 2;
    if (list.commands().back().bounds.x != 4095) return 3;
    list.reset();
    if (!list.commands().empty() || list.overflowed()) return 4;
    bestgui::DrawList empty(std::span<bestgui::Quad>{});
    if (empty.rectangle({}, {}) || !empty.overflowed()) return 5;

    constexpr int warmup = 2000, samples = 20000;
    std::vector<double> timings;
    timings.reserve(samples);
    for (int sample = -warmup; sample < samples; ++sample) {
        const auto start = Clock::now();
        list.reset();
        for (int i = 0; i < 4096; ++i)
            list.rectangle({float(i % 64) * 10, float(i / 64) * 10,
                            9, 9}, {0.25f, 0.5f, 0.75f, 1});
        observe(list.commands().data());
        const auto end = Clock::now();
        if (sample >= 0)
            timings.push_back(std::chrono::duration<double, std::micro>(end - start).count());
    }
    std::sort(timings.begin(), timings.end());
    const auto percentile = [&](double p) {
        return timings[static_cast<std::size_t>(p * (samples - 1))];
    };
    std::printf("CPU draw-list construction only; 4096 quads, %d samples\n"
                "p50 %.3f us | p95 %.3f us | p99 %.3f us | max %.3f us\n"
                "command storage %zu bytes; no window, GPU, text, or input measured\n",
                samples, percentile(.5), percentile(.95), percentile(.99),
                timings.back(), sizeof(storage));
}
