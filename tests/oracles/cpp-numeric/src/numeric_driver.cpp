// Active fixed-point conformance driver. Reads versioned numeric operations
// and writes one canonical result line per operation. Mirrors the Rust driver
// output byte for byte for the shared corpus.

#include "numeric.h"

#include <cstdint>
#include <exception>
#include <iostream>
#include <sstream>
#include <string>

int main() {
  std::string line;
  while (std::getline(std::cin, line)) {
    std::istringstream stream(line);
    std::string op;
    if (!(stream >> op)) {
      continue;
    }
    try {
      if (op == "parse") {
        std::string text;
        uint16_t scale = 0;
        stream >> text >> scale;
        const int64_t value = eng::parse_scaled(text, static_cast<uint8_t>(scale));
        std::cout << "parse " << value << "\n";
      } else if (op == "format") {
        int64_t raw = 0;
        uint16_t scale = 0;
        stream >> raw >> scale;
        const std::string text = eng::format_scaled(raw, static_cast<uint8_t>(scale));
        std::cout << "format " << text << "\n";
      } else if (op == "rescale") {
        int64_t raw = 0;
        uint16_t from = 0;
        uint16_t to = 0;
        stream >> raw >> from >> to;
        const int64_t value =
            eng::rescale_exact(raw, static_cast<uint8_t>(from), static_cast<uint8_t>(to));
        std::cout << "rescale " << value << "\n";
      } else {
        std::cerr << "unknown op: " << op << "\n";
        return 1;
      }
    } catch (const std::exception&) {
      std::cout << "error " << op << "\n";
    }
  }
  return 0;
}
