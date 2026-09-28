#include "link_frame.h"

#include <string.h>

uint16_t link_crc16(uint16_t crc, const uint8_t *data, size_t len)
{
    for (size_t i = 0; i < len; i++) {
        crc ^= (uint16_t) data[i] << 8;
        for (int b = 0; b < 8; b++) {
            crc = (crc & 0x8000) ? (uint16_t) ((crc << 1) ^ 0x1021) : (uint16_t) (crc << 1);
        }
    }
    return crc;
}

/* COBS encoder state: `code` is where the current block's length byte goes. */
typedef struct {
    uint8_t *out;
    size_t pos;
    size_t code;
    uint8_t run;
} cobs_t;

static void cobs_begin(cobs_t *c, uint8_t *out)
{
    c->out = out;
    c->code = 0;
    c->pos = 1;
    c->run = 1;
}

static void cobs_put(cobs_t *c, uint8_t byte)
{
    if (byte != 0) {
        c->out[c->pos++] = byte;
        c->run++;
    }
    if (byte == 0 || c->run == 0xFF) {
        c->out[c->code] = c->run;
        c->code = c->pos++;
        c->run = 1;
    }
}

static size_t cobs_end(cobs_t *c)
{
    c->out[c->code] = c->run;
    return c->pos;
}

size_t link_frame_encode(uint8_t chan, const uint8_t *payload, size_t len, uint8_t *out)
{
    const uint8_t head[3] = {chan, (uint8_t) (len & 0xFF), (uint8_t) (len >> 8)};
    uint16_t crc = link_crc16(0xFFFF, head, sizeof(head));
    crc = link_crc16(crc, payload, len);
    const uint8_t tail[2] = {(uint8_t) (crc & 0xFF), (uint8_t) (crc >> 8)};

    out[0] = 0x00;
    cobs_t c;
    cobs_begin(&c, out + 1);
    for (size_t i = 0; i < sizeof(head); i++) {
        cobs_put(&c, head[i]);
    }
    for (size_t i = 0; i < len; i++) {
        cobs_put(&c, payload[i]);
    }
    for (size_t i = 0; i < sizeof(tail); i++) {
        cobs_put(&c, tail[i]);
    }
    size_t n = 1 + cobs_end(&c);
    out[n++] = 0x00;
    return n;
}

bool link_frame_decode(uint8_t *buf, size_t n, uint8_t *chan, uint8_t **payload, size_t *len)
{
    // Decode in place: the output never runs ahead of the input.
    size_t in = 0;
    size_t out = 0;
    while (in < n) {
        uint8_t code = buf[in++];
        if (code == 0 || in + code - 1 > n) {
            return false;
        }
        for (uint8_t i = 1; i < code; i++) {
            buf[out++] = buf[in++];
        }
        if (code != 0xFF && in < n) {
            buf[out++] = 0x00;
        }
    }
    if (out < LINK_FRAME_OVERHEAD) {
        return false;
    }
    size_t body = out - 2;
    size_t plen = buf[1] | ((size_t) buf[2] << 8);
    if (plen != body - 3 || plen > LINK_MAX_PAYLOAD) {
        return false;
    }
    uint16_t want = buf[body] | ((uint16_t) buf[body + 1] << 8);
    if (link_crc16(0xFFFF, buf, body) != want) {
        return false;
    }
    *chan = buf[0];
    *payload = buf + 3;
    *len = plen;
    return true;
}
