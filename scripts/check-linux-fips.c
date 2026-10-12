/* Build-only probe: run separately for each Rocky crypto module. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifdef TEST_GCRYPT
#include <gcrypt.h>
#elif defined(TEST_GNUTLS)
#include <gnutls/gnutls.h>
#include <gnutls/crypto.h>
#else
#include <openssl/crypto.h>
#include <openssl/rand.h>
#endif

int main(int argc, char **argv) {
    Dl_info info;
    char *loaded;
    if (argc != 2) return 2;
#ifdef TEST_GCRYPT
    void *symbol = dlsym(RTLD_DEFAULT, "gcry_check_version");
#elif defined(TEST_GNUTLS)
    void *symbol = dlsym(RTLD_DEFAULT, "gnutls_global_init");
#else
    void *symbol = dlsym(RTLD_DEFAULT, "FIPS_mode");
#endif
    if (!symbol || !dladdr(symbol, &info) || !(loaded = realpath(info.dli_fname, NULL))) return 3;
    if (strcmp(loaded, argv[1]) != 0) {
        fprintf(stderr, "Unexpected crypto module: %s\n", loaded);
        free(loaded);
        return 4;
    }
    printf("Checking packaged module: %s\n", loaded);
    free(loaded);
#ifdef TEST_GCRYPT
    if (!gcry_check_version(NULL) || !gcry_fips_mode_active() ||
        gcry_control(GCRYCTL_SELFTEST, 0) != 0) return 5;
#elif defined(TEST_GNUTLS)
    unsigned char random_bytes[32];
    if (gnutls_global_init() != 0 ||
        gnutls_fips140_mode_enabled() != GNUTLS_FIPS140_STRICT ||
        gnutls_rnd(GNUTLS_RND_RANDOM, random_bytes, sizeof(random_bytes)) != 0) return 5;
    gnutls_global_deinit();
#else
    unsigned char random_bytes[32];
    if (FIPS_mode() != 1 || RAND_bytes(random_bytes, sizeof(random_bytes)) != 1) return 5;
#endif
    puts("Packaged crypto module passed in FIPS mode.");
    return 0;
}
