#include "metrics_io.h"

#include "esp_log.h"
#include "metrics_model.h"
#include "metrics_parser.h"

static const char *TAG = "metrics_io";

void metrics_io_handle(uint8_t *payload, size_t len)
{
    metrics_snapshot_t snap;
    if (metrics_parse_line((const char *) payload, &snap) != ESP_OK) {
        ESP_LOGW(TAG, "ignored snapshot");
        return;
    }
    metrics_model_set(&snap);
    ESP_LOGD(TAG, "cpu %dC gpu %dC", (int) (snap.cpu.temp_c + 0.5f), (int) (snap.gpu.temp_c + 0.5f));
}
