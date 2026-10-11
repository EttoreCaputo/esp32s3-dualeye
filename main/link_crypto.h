#pragma once

/* The WiFi link's pairing and encryption (docs/protocol.md, "WiFi"): proofs
 * that both ends hold the pairing key, then AES-256-GCM on every frame with a
 * key per direction, derived from the pairing key and both nonces. */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "psa/crypto.h"

#ifdef __cplusplus
extern "C" {
#endif

#define LINK_KEY_BYTES 32
#define LINK_NONCE_BYTES 16
#define LINK_PROOF_BYTES 32
/* The GCM tag after each sealed payload. */
#define LINK_SEAL_OVERHEAD 16

typedef struct {
    psa_key_id_t tx;
    psa_key_id_t rx;
    uint64_t tx_count;
    uint64_t rx_count;
} link_cipher_t;

/** HMAC-SHA256(key, label || nb || nh), the proofs and session keys. */
bool link_crypto_derive(const uint8_t key[LINK_KEY_BYTES], const char *label, const uint8_t nb[LINK_NONCE_BYTES],
                        const uint8_t nh[LINK_NONCE_BYTES], uint8_t out[32]);

/** Derive and load the session keys, board side: tx is board → host. */
bool link_cipher_start(link_cipher_t *c, const uint8_t key[LINK_KEY_BYTES], const uint8_t nb[LINK_NONCE_BYTES],
                       const uint8_t nh[LINK_NONCE_BYTES]);

void link_cipher_end(link_cipher_t *c);

/** Encrypt `len` bytes into `out` (len + LINK_SEAL_OVERHEAD), with the channel
 * as associated data. The counter only moves on with link_cipher_commit, once
 * the frame is on its way: a frame never sent may reuse it. */
bool link_seal(const link_cipher_t *c, uint8_t chan, const uint8_t *in, size_t len, uint8_t *out);
void link_cipher_commit(link_cipher_t *c);

/** Decrypt and check a sealed payload into `out` (len - LINK_SEAL_OVERHEAD). */
bool link_open(link_cipher_t *c, uint8_t chan, const uint8_t *in, size_t len, uint8_t *out, size_t *out_len);

/** Lowercase hex of `n` bytes into `out` (2n + 1). */
void link_hex(const uint8_t *in, size_t n, char *out);
/** Parse exactly 2n hex digits. */
bool link_unhex(const char *in, uint8_t *out, size_t n);

#ifdef __cplusplus
}
#endif
