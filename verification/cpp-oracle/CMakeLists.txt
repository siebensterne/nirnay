cmake_minimum_required(VERSION 3.20)
project(nirnay_cpp_numeric_oracle LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# The oracle uses __int128 for checked intermediate arithmetic. That is a
# GCC/Clang 64-bit extension; the supported matrix is explicit and enforced so
# a build host cannot silently produce an unsupported configuration.
include(CheckCXXSourceCompiles)
check_cxx_source_compiles("
int main() {
  __int128 value = static_cast<__int128>(1) << 64;
  return value > 0 ? 0 : 1;
}" NIRNAY_HAS_INT128)
if(CMAKE_SIZEOF_VOID_P LESS 8)
  message(FATAL_ERROR "The numeric oracle requires a 64-bit target.")
endif()
if(NOT NIRNAY_HAS_INT128)
  message(FATAL_ERROR
    "The numeric oracle requires a compiler with __int128 support "
    "(GCC or Clang on a 64-bit target). MSVC is not a supported build host.")
endif()

add_library(nirnay_numeric_oracle STATIC
    src/numeric.cpp
)
target_include_directories(nirnay_numeric_oracle PUBLIC src)
target_compile_options(nirnay_numeric_oracle PRIVATE -Wall -Wextra -Werror)

enable_testing()
add_executable(nirnay_numeric_oracle_test tests/numeric_orderbook_test.cpp)
target_link_libraries(nirnay_numeric_oracle_test PRIVATE nirnay_numeric_oracle)
target_compile_options(nirnay_numeric_oracle_test PRIVATE -Wall -Wextra -Werror)
add_test(NAME numeric_orderbook_oracle COMMAND nirnay_numeric_oracle_test)

# Pure numeric differential driver; no market-state corpus is consumed.
add_executable(nirnay_numeric_driver src/numeric_driver.cpp)
target_link_libraries(nirnay_numeric_driver PRIVATE nirnay_numeric_oracle)
target_compile_options(nirnay_numeric_driver PRIVATE -Wall -Wextra -Werror)
