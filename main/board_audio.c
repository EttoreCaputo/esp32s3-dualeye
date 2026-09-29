#include "board_audio.h"

#include "driver/i2c_master.h"
#include "driver/i2s_std.h"
#include "driver/i2s_tdm.h"
#include "es7210_adc.h"
#include "es8311_codec.h"
#include "esp_check.h"
#include "esp_codec_dev.h"
#include "esp_codec_dev_defaults.h"
#include "esp_log.h"

static const char *TAG = "board_audio";

#define AUDIO_DMA_DESC_NUM 6
#define AUDIO_DMA_FRAME_NUM 240
#define AUDIO_DEFAULT_VOLUME 60
/* M0: speech at 30 dB was ~25 dB below ideal; the AFE's AGC does the rest. */
#define AUDIO_MIC_GAIN_DB 37.5f
#define AUDIO_REF_GAIN_DB 30.0f

static i2c_master_bus_handle_t s_i2c;
static i2s_chan_handle_t s_tx;
static i2s_chan_handle_t s_rx;
static esp_codec_dev_handle_t s_out;
static esp_codec_dev_handle_t s_in;

static esp_err_t i2c_start(void)
{
    const i2c_master_bus_config_t cfg = {
        .i2c_port = I2C_NUM_0,
        .sda_io_num = BOARD_AUDIO_I2C_SDA_GPIO,
        .scl_io_num = BOARD_AUDIO_I2C_SCL_GPIO,
        .clk_source = I2C_CLK_SRC_DEFAULT,
        .glitch_ignore_cnt = 7,
        .flags.enable_internal_pullup = true,
    };
    return i2c_new_master_bus(&cfg, &s_i2c);
}

/* TX in standard stereo mode for the ES8311, RX in 4-slot TDM for the ES7210,
 * both on I2S0 so they share MCLK/BCLK/WS. */
static esp_err_t i2s_start(void)
{
    const i2s_chan_config_t chan_cfg = {
        .id = I2S_NUM_0,
        .role = I2S_ROLE_MASTER,
        .dma_desc_num = AUDIO_DMA_DESC_NUM,
        .dma_frame_num = AUDIO_DMA_FRAME_NUM,
        .auto_clear_after_cb = true,
        .auto_clear_before_cb = false,
        .intr_priority = 0,
    };
    ESP_RETURN_ON_ERROR(i2s_new_channel(&chan_cfg, &s_tx, &s_rx), TAG, "i2s channels");

    const i2s_std_config_t std_cfg = {
        .clk_cfg = {
            .sample_rate_hz = BOARD_AUDIO_SAMPLE_RATE,
            .clk_src = I2S_CLK_SRC_DEFAULT,
            .mclk_multiple = I2S_MCLK_MULTIPLE_256,
        },
        .slot_cfg = I2S_STD_PHILIPS_SLOT_DEFAULT_CONFIG(I2S_DATA_BIT_WIDTH_16BIT, I2S_SLOT_MODE_STEREO),
        .gpio_cfg = {
            .mclk = BOARD_AUDIO_I2S_MCLK_GPIO,
            .bclk = BOARD_AUDIO_I2S_BCLK_GPIO,
            .ws = BOARD_AUDIO_I2S_WS_GPIO,
            .dout = BOARD_AUDIO_I2S_DOUT_GPIO,
            .din = I2S_GPIO_UNUSED,
        },
    };

    i2s_tdm_config_t tdm_cfg = {
        .clk_cfg = {
            .sample_rate_hz = BOARD_AUDIO_SAMPLE_RATE,
            .clk_src = I2S_CLK_SRC_DEFAULT,
            .mclk_multiple = I2S_MCLK_MULTIPLE_256,
            .bclk_div = 8,
        },
        .slot_cfg = I2S_TDM_PHILIPS_SLOT_DEFAULT_CONFIG(I2S_DATA_BIT_WIDTH_16BIT, I2S_SLOT_MODE_STEREO,
                                                        I2S_TDM_SLOT0 | I2S_TDM_SLOT1 | I2S_TDM_SLOT2
                                                            | I2S_TDM_SLOT3),
        .gpio_cfg = {
            .mclk = BOARD_AUDIO_I2S_MCLK_GPIO,
            .bclk = BOARD_AUDIO_I2S_BCLK_GPIO,
            .ws = BOARD_AUDIO_I2S_WS_GPIO,
            .dout = I2S_GPIO_UNUSED,
            .din = BOARD_AUDIO_I2S_DIN_GPIO,
        },
    };

    ESP_RETURN_ON_ERROR(i2s_channel_init_std_mode(s_tx, &std_cfg), TAG, "i2s tx");
    ESP_RETURN_ON_ERROR(i2s_channel_init_tdm_mode(s_rx, &tdm_cfg), TAG, "i2s rx");
    ESP_RETURN_ON_ERROR(i2s_channel_enable(s_tx), TAG, "i2s tx enable");
    ESP_RETURN_ON_ERROR(i2s_channel_enable(s_rx), TAG, "i2s rx enable");
    return ESP_OK;
}

static esp_err_t codecs_start(void)
{
    audio_codec_i2s_cfg_t i2s_cfg = {
        .port = I2S_NUM_0,
        .rx_handle = s_rx,
        .tx_handle = s_tx,
    };
    const audio_codec_data_if_t *data_if = audio_codec_new_i2s_data(&i2s_cfg);
    ESP_RETURN_ON_FALSE(data_if != NULL, ESP_FAIL, TAG, "i2s data if");

    audio_codec_i2c_cfg_t i2c_cfg = {
        .port = I2C_NUM_0,
        .addr = ES8311_CODEC_DEFAULT_ADDR,
        .bus_handle = s_i2c,
    };
    const audio_codec_ctrl_if_t *out_ctrl = audio_codec_new_i2c_ctrl(&i2c_cfg);
    const audio_codec_gpio_if_t *gpio_if = audio_codec_new_gpio();
    ESP_RETURN_ON_FALSE(out_ctrl != NULL && gpio_if != NULL, ESP_FAIL, TAG, "es8311 ctrl");

    es8311_codec_cfg_t es8311_cfg = {
        .ctrl_if = out_ctrl,
        .gpio_if = gpio_if,
        .codec_mode = ESP_CODEC_DEV_WORK_MODE_DAC,
        .pa_pin = BOARD_AUDIO_PA_GPIO,
        .use_mclk = true,
        .hw_gain = {
            .pa_voltage = 5.0f,
            .codec_dac_voltage = 3.3f,
        },
    };
    const audio_codec_if_t *out_codec = es8311_codec_new(&es8311_cfg);
    ESP_RETURN_ON_FALSE(out_codec != NULL, ESP_FAIL, TAG, "es8311 not answering on I2C");

    esp_codec_dev_cfg_t out_cfg = {
        .dev_type = ESP_CODEC_DEV_TYPE_OUT,
        .codec_if = out_codec,
        .data_if = data_if,
    };
    s_out = esp_codec_dev_new(&out_cfg);
    ESP_RETURN_ON_FALSE(s_out != NULL, ESP_FAIL, TAG, "output dev");

    i2c_cfg.addr = ES7210_CODEC_DEFAULT_ADDR;
    const audio_codec_ctrl_if_t *in_ctrl = audio_codec_new_i2c_ctrl(&i2c_cfg);
    ESP_RETURN_ON_FALSE(in_ctrl != NULL, ESP_FAIL, TAG, "es7210 ctrl");

    es7210_codec_cfg_t es7210_cfg = {
        .ctrl_if = in_ctrl,
        .mic_selected = ES7210_SEL_MIC1 | ES7210_SEL_MIC2 | ES7210_SEL_MIC3 | ES7210_SEL_MIC4,
    };
    const audio_codec_if_t *in_codec = es7210_codec_new(&es7210_cfg);
    ESP_RETURN_ON_FALSE(in_codec != NULL, ESP_FAIL, TAG, "es7210 not answering on I2C");

    esp_codec_dev_cfg_t in_cfg = {
        .dev_type = ESP_CODEC_DEV_TYPE_IN,
        .codec_if = in_codec,
        .data_if = data_if,
    };
    s_in = esp_codec_dev_new(&in_cfg);
    ESP_RETURN_ON_FALSE(s_in != NULL, ESP_FAIL, TAG, "input dev");

    esp_codec_dev_sample_info_t out_fs = {
        .bits_per_sample = 16,
        .channel = 1,
        .sample_rate = BOARD_AUDIO_SAMPLE_RATE,
    };
    ESP_RETURN_ON_FALSE(esp_codec_dev_open(s_out, &out_fs) == ESP_CODEC_DEV_OK, ESP_FAIL, TAG, "open out");

    esp_codec_dev_sample_info_t in_fs = {
        .bits_per_sample = 16,
        .channel = BOARD_AUDIO_IN_CHANNELS,
        .sample_rate = BOARD_AUDIO_SAMPLE_RATE,
    };
    ESP_RETURN_ON_FALSE(esp_codec_dev_open(s_in, &in_fs) == ESP_CODEC_DEV_OK, ESP_FAIL, TAG, "open in");
    return ESP_OK;
}

esp_err_t board_audio_init(void)
{
    ESP_RETURN_ON_ERROR(i2c_start(), TAG, "i2c");
    ESP_RETURN_ON_ERROR(i2s_start(), TAG, "i2s");
    ESP_RETURN_ON_ERROR(codecs_start(), TAG, "codecs");
    ESP_RETURN_ON_ERROR(board_audio_set_volume(AUDIO_DEFAULT_VOLUME), TAG, "volume");
    ESP_RETURN_ON_ERROR(board_audio_set_in_gain(AUDIO_REF_GAIN_DB), TAG, "gain");
    ESP_RETURN_ON_FALSE(esp_codec_dev_set_in_channel_gain(s_in, ESP_CODEC_DEV_MAKE_CHANNEL_MASK(BOARD_AUDIO_MIC_SLOT),
                                                          AUDIO_MIC_GAIN_DB)
                            == ESP_CODEC_DEV_OK,
                        ESP_FAIL, TAG, "mic gain");
    ESP_RETURN_ON_ERROR(board_audio_set_mute(true), TAG, "mute");
    ESP_LOGI(TAG, "ES8311 + ES7210 ready, %d Hz", BOARD_AUDIO_SAMPLE_RATE);
    return ESP_OK;
}

esp_err_t board_audio_write(const int16_t *samples, size_t count)
{
    int ret = esp_codec_dev_write(s_out, (void *) samples, (int) (count * sizeof(int16_t)));
    return ret == ESP_CODEC_DEV_OK ? ESP_OK : ESP_FAIL;
}

esp_err_t board_audio_read(int16_t *frames_out, size_t frames)
{
    int bytes = (int) (frames * BOARD_AUDIO_IN_CHANNELS * sizeof(int16_t));
    return esp_codec_dev_read(s_in, frames_out, bytes) == ESP_CODEC_DEV_OK ? ESP_OK : ESP_FAIL;
}

esp_err_t board_audio_set_volume(int volume)
{
    return esp_codec_dev_set_out_vol(s_out, volume) == ESP_CODEC_DEV_OK ? ESP_OK : ESP_FAIL;
}

esp_err_t board_audio_set_mute(bool mute)
{
    return esp_codec_dev_set_out_mute(s_out, mute) == ESP_CODEC_DEV_OK ? ESP_OK : ESP_FAIL;
}

esp_err_t board_audio_set_in_gain(float db)
{
    return esp_codec_dev_set_in_gain(s_in, db) == ESP_CODEC_DEV_OK ? ESP_OK : ESP_FAIL;
}
