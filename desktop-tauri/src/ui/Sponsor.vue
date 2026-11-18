<script setup lang="ts">
import { ref } from "vue";
import { t } from "../i18n";
import type { AgentPort } from "../port/port";
import wechatSrc from "../assets/wechat-pay.jpg";
import alipaySrc from "../assets/alipay.jpg";
import { COMMUNITY } from "./communityLinks";

const props = defineProps<{ port: Pick<AgentPort, "openExternal"> }>();
const qr = ref(false);
const failed = ref("");

const open = (url: string) =>
  void props.port.openExternal(url).then(() => { failed.value = ""; }, () => { failed.value = t("common.could_not_open_a_browser", { at: url }); });
</script>

<template>
  <div class="spon">
    <p class="spon-note">{{ t("feedback.if_reasonix_has_saved_you") }}</p>
    <div class="spon-acts">
      <a class="spon-link" :href="COMMUNITY.paypal" data-action="sponsor.paypal" @click.prevent="open(COMMUNITY.paypal)">PayPal</a>
      <button type="button" class="btn sm" data-action="sponsor.qr" :aria-expanded="qr" :aria-controls="qr ? 'spon-qr' : undefined" @click="qr = !qr">
        {{ qr ? t("feedback.hide_payment_qr_codes") : t("feedback.show_payment_qr_codes") }}
      </button>
    </div>
    <div v-if="qr" id="spon-qr" class="spon-qr">
      <figure>
        <img :src="wechatSrc" :alt="t('feedback.wechat_pay_qr_code')" width="180" />
        <figcaption>{{ t("feedback.wechat_pay") }}</figcaption>
      </figure>
      <figure>
        <img :src="alipaySrc" :alt="t('feedback.alipay_qr_code')" width="180" />
        <figcaption>{{ t("feedback.alipay") }}</figcaption>
      </figure>
    </div>
    <p v-if="failed" class="comm-fail" role="alert">{{ failed }}</p>
  </div>
</template>