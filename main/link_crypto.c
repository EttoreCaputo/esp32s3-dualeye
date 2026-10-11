#include "link_crypto.h"

#include <string.h>

#define GCM_NONCE_BYTES 12

bool link_crypto_derive(const uint8_t key[LINK_KEY_BYTES], const char *label, const uint8_t nb[LINK_NONCE_BYTES],
                        const uint8_t nh[LINK_NONCE_BYTES], uint8_t out[32])
{
    uint8_t msg[32 + 2 * LINK_NONCE_BYTES];
    size_t n = strlen(label);
    if (n > 32) {
        return false;
    }
    memcpy(msg, label, n);
    memcpy(msg + n, nb, LINK_NONCE_BYTES);
    memcpy(msg + n + LINK_NONCE_BYTES, nh, LINK_NONCE_BYTES);
    n += 2 * LINK_NONCE_BYTES;

    if (psa_crypto_init() != PSA_SUCCESS) {
        return false;
    }
    psa_key_attributes_t attr = PSA_KEY_ATTRIBUTES_INIT;
    psa_set_key_type(&attr, PSA_KEY_TYPE_HMAC);
    psa_set_key_bits(&attr, LINK_KEY_BYTES * 8);
    psa_set_key_algorithm(&attr, PSA_ALG_HMAC(PSA_ALG_SHA_256));
    psa_set_key_usage_flags(&attr, PSA_KEY_USAGE_SIGN_MESSAGE);
    psa_key_id_t id = PSA_KEY_ID_NULL;
    if (psa_import_key(&attr, key, LINK_KEY_BYTES, &id) != PSA_SUCCESS) {
        return false;
    }
    size_t len = 0;
    psa_status_t st = psa_mac_compute(id, PSA_ALG_HMAC(PSA_ALG_SHA_256), msg, n, out, 32, &len);
    psa_destroy_key(id);
    return st == PSA_SUCCESS && len == 32;
}

static bool load_aes(const uint8_t raw[32], psa_key_id_t *id)
{
    psa_key_attributes_t attr = PSA_KEY_ATTRIBUTES_INIT;
    psa_set_key_type(&attr, PSA_KEY_TYPE_AES);
    psa_set_key_bits(&attr, 256);
    psa_set_key_algorithm(&attr, PSA_ALG_GCM);
    psa_set_key_usage_flags(&attr, PSA_KEY_USAGE_ENCRYPT | PSA_KEY_USAGE_DECRYPT);
    return psa_import_key(&attr, raw, 32, id) == PSA_SUCCESS;
}

bool link_cipher_start(link_cipher_t *c, const uint8_t key[LINK_KEY_BYTES], const uint8_t nb[LINK_NONCE_BYTES],
                       const uint8_t nh[LINK_NONCE_BYTES])
{
    memset(c, 0, sizeof(*c));
    uint8_t b2h[32];
    uint8_t h2b[32];
    bool ok = link_crypto_derive(key, "dualeye b2h", nb, nh, b2h) && link_crypto_derive(key, "dualeye h2b", nb, nh, h2b)
              && load_aes(b2h, &c->tx) && load_aes(h2b, &c->rx);
    memset(b2h, 0, sizeof(b2h));
    memset(h2b, 0, sizeof(h2b));
    if (!ok) {
        link_cipher_end(c);
    }
    return ok;
}

void link_cipher_end(link_cipher_t *c)
{
    if (c->tx != PSA_KEY_ID_NULL) {
        psa_destroy_key(c->tx);
    }
    if (c->rx != PSA_KEY_ID_NULL) {
        psa_destroy_key(c->rx);
    }
    memset(c, 0, sizeof(*c));
}

/** Four zero bytes, then the frame counter, little-endian. */
static void gcm_nonce(uint64_t count, uint8_t nonce[GCM_NONCE_BYTES])
{
    memset(nonce, 0, GCM_NONCE_BYTES);
    for (int i = 0; i < 8; i++) {
        nonce[4 + i] = (uint8_t) (count >> (8 * i));
    }
}

bool link_seal(const link_cipher_t *c, uint8_t chan, const uint8_t *in, size_t len, uint8_t *out)
{
    uint8_t nonce[GCM_NONCE_BYTES];
    gcm_nonce(c->tx_count, nonce);
    size_t n = 0;
    psa_status_t st = psa_aead_encrypt(c->tx, PSA_ALG_GCM, nonce, sizeof(nonce), &chan, 1, in, len, out,
                                       len + LINK_SEAL_OVERHEAD, &n);
    return st == PSA_SUCCESS && n == len + LINK_SEAL_OVERHEAD;
}

void link_cipher_commit(link_cipher_t *c)
{
    c->tx_count++;
}

bool link_open(link_cipher_t *c, uint8_t chan, const uint8_t *in, size_t len, uint8_t *out, size_t *out_len)
{
    if (len < LINK_SEAL_OVERHEAD) {
        return false;
    }
    uint8_t nonce[GCM_NONCE_BYTES];
    gcm_nonce(c->rx_count, nonce);
    psa_status_t st = psa_aead_decrypt(c->rx, PSA_ALG_GCM, nonce, sizeof(nonce), &chan, 1, in, len, out,
                                       len - LINK_SEAL_OVERHEAD, out_len);
    if (st != PSA_SUCCESS) {
        return false;
    }
    c->rx_count++;
    return true;
}

void link_hex(const uint8_t *in, size_t n, char *out)
{
    static const char digits[] = "0123456789abcdef";
    for (size_t i = 0; i < n; i++) {
        out[2 * i] = digits[in[i] >> 4];
        out[2 * i + 1] = digits[in[i] & 0x0F];
    }
    out[2 * n] = '\0';
}

static int hex_value(char c)
{
    if (c >= '0' && c <= '9') {
        return c - '0';
    }
    if (c >= 'a' && c <= 'f') {
        return c - 'a' + 10;
    }
    if (c >= 'A' && c <= 'F') {
        return c - 'A' + 10;
    }
    return -1;
}

bool link_unhex(const char *in, uint8_t *out, size_t n)
{
    if (in == NULL || strlen(in) != 2 * n) {
        return false;
    }
    for (size_t i = 0; i < n; i++) {
        int hi = hex_value(in[2 * i]);
        int lo = hex_value(in[2 * i + 1]);
        if (hi < 0 || lo < 0) {
            return false;
        }
        out[i] = (uint8_t) (hi << 4 | lo);
    }
    return true;
}
