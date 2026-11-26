#include "numeric.h"

#include <cctype>
#include <cmath>
#include <numeric>

namespace eng {

namespace {

// Checked intermediate arithmetic uses the GCC/Clang 128-bit extension.
// Marking it once keeps the strict -Wpedantic contract intact at every use
// site without suppressing a diagnostic.
__extension__ typedef __int128 wide_int;

uint64_t magnitude_of(int64_t value) {
  return value < 0 ? static_cast<uint64_t>(-(value + 1)) + 1 : static_cast<uint64_t>(value);
}

int64_t gcd_abs(int64_t a, int64_t b) {
  // Unsigned magnitude: negating INT64_MIN would be undefined behavior.
  uint64_t x = magnitude_of(a);
  uint64_t y = magnitude_of(b);
  return static_cast<int64_t>(std::gcd(x, y));
}

}  // namespace

Ratio::Ratio(int64_t numerator_, int64_t denominator_) {
  if (denominator_ == 0)
    throw std::invalid_argument("zero ratio denominator");
  if (denominator_ < 0) {
    // Checked negation: INT64_MIN has no positive i64 negation.
    if (denominator_ == INT64_MIN) {
      throw std::overflow_error("ratio denominator INT64_MIN cannot be normalized");
    }
    if (numerator_ == INT64_MIN) {
      throw std::overflow_error("ratio numerator INT64_MIN cannot be normalized");
    }
    numerator_ = static_cast<int64_t>(-(numerator_ + 1));
    denominator_ = -denominator_;
  }
  int64_t divisor = gcd_abs(numerator_, denominator_);
  if (divisor <= 0)
    divisor = 1;
  numerator = numerator_ / divisor;
  denominator = denominator_ / divisor;
}

int64_t Ratio::apply_exact(int64_t value) const {
  wide_int product = static_cast<wide_int>(value) * static_cast<wide_int>(numerator);
  wide_int divisor = static_cast<wide_int>(denominator);
  if (product % divisor != 0)
    throw std::invalid_argument("ratio result is not exact");
  wide_int result = product / divisor;
  if (result < static_cast<wide_int>(INT64_MIN) || result > static_cast<wide_int>(INT64_MAX)) {
    throw std::overflow_error("ratio result outside i64");
  }
  return static_cast<int64_t>(result);
}

int64_t scale_factor(uint8_t scale) {
  if (scale > kMaxScale)
    throw std::invalid_argument("scale exceeds maximum 18");
  int64_t factor = 1;
  for (uint8_t i = 0; i < scale; ++i)
    factor *= 10;
  return factor;
}

int64_t parse_scaled(const std::string& input, uint8_t scale) {
  if (scale > kMaxScale)
    throw std::invalid_argument("scale exceeds maximum 18");
  // Trim whitespace.
  size_t begin = 0;
  while (begin < input.size() && std::isspace(static_cast<unsigned char>(input[begin])))
    ++begin;
  size_t end = input.size();
  while (end > begin && std::isspace(static_cast<unsigned char>(input[end - 1])))
    --end;
  if (begin >= end)
    throw std::invalid_argument("empty fixed-point value");
  std::string text = input.substr(begin, end - begin);

  bool negative = false;
  size_t pos = 0;
  if (text[pos] == '-') {
    negative = true;
    ++pos;
  } else if (text[pos] == '+') {
    ++pos;
  }
  if (pos >= text.size())
    throw std::invalid_argument("missing digits");
  size_t dot = text.find('.', pos);
  std::string whole = text.substr(pos, dot == std::string::npos ? std::string::npos : dot - pos);
  std::string fraction = dot == std::string::npos ? "" : text.substr(dot + 1);
  if (text.find('.', dot == std::string::npos ? pos : dot + 1) != std::string::npos) {
    throw std::invalid_argument("multiple decimal points");
  }
  if (whole.empty() && fraction.empty())
    throw std::invalid_argument("missing digits");
  for (char c : whole) {
    if (!std::isdigit(static_cast<unsigned char>(c)))
      throw std::invalid_argument("invalid digits");
  }
  for (char c : fraction) {
    if (!std::isdigit(static_cast<unsigned char>(c)))
      throw std::invalid_argument("invalid digits");
  }
  if (fraction.size() > scale)
    throw std::invalid_argument("too many fractional digits");

  wide_int whole_value = 0;
  for (char c : whole) {
    whole_value = whole_value * 10 + (c - '0');
    if (whole_value > static_cast<wide_int>(INT64_MAX) + 1) {
      throw std::overflow_error("whole part out of range");
    }
  }
  wide_int fraction_value = 0;
  if (!fraction.empty()) {
    for (char c : fraction)
      fraction_value = fraction_value * 10 + (c - '0');
    int64_t pad = static_cast<int64_t>(scale) - static_cast<int64_t>(fraction.size());
    wide_int factor = 1;
    for (int64_t i = 0; i < pad; ++i)
      factor *= 10;
    fraction_value *= factor;
  }
  wide_int factor = 1;
  for (uint8_t i = 0; i < scale; ++i)
    factor *= 10;
  wide_int magnitude = whole_value * factor + fraction_value;
  wide_int signed_value = negative ? -magnitude : magnitude;
  if (signed_value < static_cast<wide_int>(INT64_MIN) ||
      signed_value > static_cast<wide_int>(INT64_MAX)) {
    throw std::overflow_error("fixed-point value out of range");
  }
  return static_cast<int64_t>(signed_value);
}

int64_t parse_quantity_scaled(const std::string& input, uint8_t scale) {
  int64_t value = parse_scaled(input, scale);
  if (value < 0)
    throw std::invalid_argument("negative quantity");
  return value;
}

std::string format_scaled(int64_t value, uint8_t scale) {
  int64_t factor = scale_factor(scale);
  std::string sign = value < 0 ? "-" : "";
  uint64_t magnitude =
      value < 0 ? static_cast<uint64_t>(-(value + 1)) + 1 : static_cast<uint64_t>(value);
  uint64_t whole = magnitude / static_cast<uint64_t>(factor);
  uint64_t fraction = magnitude % static_cast<uint64_t>(factor);
  if (fraction == 0)
    return sign + std::to_string(whole);
  std::string digits = std::to_string(fraction);
  digits = std::string(scale - digits.size(), '0') + digits;
  while (!digits.empty() && digits.back() == '0')
    digits.pop_back();
  return sign + std::to_string(whole) + "." + digits;
}

int64_t rescale_exact(int64_t value, uint8_t from_scale, uint8_t to_scale) {
  if (from_scale > kMaxScale || to_scale > kMaxScale) {
    throw std::invalid_argument("scale exceeds maximum 18");
  }
  wide_int input = static_cast<wide_int>(value);
  wide_int output;
  if (to_scale >= from_scale) {
    wide_int factor = 1;
    for (uint8_t i = 0; i < static_cast<uint8_t>(to_scale - from_scale); ++i)
      factor *= 10;
    output = input * factor;
  } else {
    wide_int divisor = 1;
    for (uint8_t i = 0; i < static_cast<uint8_t>(from_scale - to_scale); ++i)
      divisor *= 10;
    if (input % divisor != 0)
      throw std::invalid_argument("value is not exactly representable");
    output = input / divisor;
  }
  if (output < static_cast<wide_int>(INT64_MIN) || output > static_cast<wide_int>(INT64_MAX)) {
    throw std::overflow_error("rescaled value outside i64");
  }
  return static_cast<int64_t>(output);
}

}  // namespace eng
