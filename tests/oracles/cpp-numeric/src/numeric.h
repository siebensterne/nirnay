#pragma once

#include <cstdint>
#include <stdexcept>
#include <string>

namespace eng {

// Maximum supported decimal scale for an i64 coefficient. 10^18 fits, 10^19
// does not.
inline constexpr uint8_t kMaxScale = 18;

// Signed i64 coefficient. The scale and unit live in NumericSpec, not in the
// value.
struct Price {
  int64_t raw = 0;
  explicit constexpr Price(int64_t value = 0) : raw(value) {}
  constexpr auto operator<=>(const Price&) const = default;
  constexpr bool operator==(const Price&) const = default;
};

// Non-negative by construction.
struct Quantity {
  int64_t raw = 0;
  Quantity() = default;
  explicit Quantity(int64_t value) : raw(value) {
    if (value < 0)
      throw std::invalid_argument("negative quantity");
  }
  constexpr auto operator<=>(const Quantity&) const = default;
  constexpr bool operator==(const Quantity&) const = default;
};

// Signed delta, distinct from absolute Quantity.
struct QuantityDelta {
  int64_t raw = 0;
  explicit constexpr QuantityDelta(int64_t value = 0) : raw(value) {}
  constexpr auto operator<=>(const QuantityDelta&) const = default;
  constexpr bool operator==(const QuantityDelta&) const = default;
};

// Exact rational multiplier (contract multiplier, lot conversion). No floating
// point.
struct Ratio {
  int64_t numerator = 1;
  int64_t denominator = 1;
  Ratio() = default;
  Ratio(int64_t numerator_, int64_t denominator_);
  int64_t apply_exact(int64_t value) const;
};

int64_t scale_factor(uint8_t scale);
int64_t parse_scaled(const std::string& input, uint8_t scale);
int64_t parse_quantity_scaled(const std::string& input, uint8_t scale);
std::string format_scaled(int64_t value, uint8_t scale);
int64_t rescale_exact(int64_t value, uint8_t from_scale, uint8_t to_scale);

}  // namespace eng
