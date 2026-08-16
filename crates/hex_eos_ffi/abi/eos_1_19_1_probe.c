#include <eos_sdk.h>

#if !defined(EOS_MAJOR_VERSION) || !defined(EOS_MINOR_VERSION) || \
    !defined(EOS_PATCH_VERSION)
#error "official EOS SDK version macros are unavailable"
#endif

#if EOS_MAJOR_VERSION != 1 || EOS_MINOR_VERSION != 19 || EOS_PATCH_VERSION != 1
#error "official EOS headers do not match the pinned 1.19.1 baseline"
#endif

/*
 * This is the complete C ABI surface declared by the first foundation commit.
 * Assignment is a compile-time function-signature/calling-convention check. Add
 * _Static_assert(sizeof(...)) and _Alignof(...) checks here before introducing
 * every layout-bearing Rust declaration.
 */
static const char *(EOS_CALL *hex_expected_get_version)(void) = EOS_GetVersion;

int main(void) {
    return hex_expected_get_version == 0;
}
