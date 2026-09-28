#include "metrics_io.h"

#include <stdio.h>
#include <string.h>

#include "audio_selftest.h"
#include "esp_app_desc.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "metrics_model.h"
#include "metrics_parser.h"

#define METRICS_IO_LINE_MAX 1024

static const char *TAG = "metrics_io";

void metrics_io_report_version(void)
{
    const esp_app_desc_t *app = esp_app_get_description();
    printf("{\"dualeye\":\"%s\",\"idf\":\"%s\"}\n", app->version, app->idf_ver);
    fflush(stdout);
}

static void metrics_io_task(void *arg)
{
    static char line[METRICS_IO_LINE_MAX];
    size_t len = 0;
    bool overflow = false;

    setvbuf(stdin, NULL, _IONBF, 0);
    ESP_LOGI(TAG, "Reading snapshot JSON from USB serial");

    while (true) {
        int c = fgetc(stdin);
        if (c == EOF) {
            clearerr(stdin);
            vTaskDelay(pdMS_TO_TICKS(20));
            continue;
        }
        if (c == '\r') {
            continue;
        }
        if (c == '\n') {
            line[len] = '\0';
            if (!overflow && len == sizeof(METRICS_IO_VERSION_QUERY) - 1
                && memcmp(line, METRICS_IO_VERSION_QUERY, len) == 0) {
                metrics_io_report_version();
            } else if (!overflow && audio_selftest_command(line)) {
                // handled
            } else if (!overflow && len > 0 && line[0] == '{') {
                metrics_snapshot_t snap;
                esp_err_t err = metrics_parse_line(line, &snap);
                if (err == ESP_OK) {
                    metrics_model_set(&snap);
                    ESP_LOGI(TAG, "cpu %dC gpu %dC", (int) (snap.cpu.temp_c + 0.5f),
                             (int) (snap.gpu.temp_c + 0.5f));
                } else {
                    ESP_LOGW(TAG, "ignored metrics line");
                }
            } else if (overflow) {
                ESP_LOGW(TAG, "dropped overlong line");
            }
            len = 0;
            overflow = false;
            continue;
        }
        if (overflow || len + 1 >= sizeof(line)) {
            overflow = true;
            continue;
        }
        line[len++] = (char) c;
    }
}

void metrics_io_start(void)
{
    BaseType_t ok = xTaskCreate(metrics_io_task, "metrics_io", 6144, NULL, 3, NULL);
    if (ok != pdPASS) {
        ESP_LOGE(TAG, "failed to start USB reader");
    }
}
