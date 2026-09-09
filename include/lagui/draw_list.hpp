#pragma once
#include <cstddef>
#include <cstdint>
#include <span>

namespace lagui {
struct Rect { float x, y, width, height; };
struct Color { float r, g, b, a; }; // Linear, premultiplied alpha.
struct Quad { Rect bounds; Color color; };

// Caller owns storage. No allocation, virtual dispatch, or OS dependency.
// Commands preserve painter's order. Overflow is reported, never reallocated.
class DrawList {
public:
    explicit DrawList(std::span<Quad> storage) noexcept : storage_(storage) {}
    void reset() noexcept { size_ = 0; overflowed_ = false; }
    bool rectangle(Rect bounds, Color color) noexcept {
        if (size_ == storage_.size()) { overflowed_ = true; return false; }
        storage_[size_++] = {bounds, color};
        return true;
    }
    std::span<const Quad> commands() const noexcept {
        return storage_.first(size_);
    }
    bool overflowed() const noexcept { return overflowed_; }
private:
    std::span<Quad> storage_;
    std::size_t size_ = 0;
    bool overflowed_ = false;
};
} // namespace lagui
