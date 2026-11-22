# Omvix Hub — Windows Kod İmzalama (Authenticode) Araştırma Raporu

Bu rapor, **Omvix Hub**'ın user-mode Windows ikilileri (`omvix-hub.exe`, `omvix-cli.exe`) ve yerleşik **İmzalı Güncelleyici** (`PINNED_SIGNER_SUBJECT` + `AuthenticodeValid` politikası) için 2025/2026 itibarıyla uygulanabilir Authenticode sertifika seçeneklerini özetler.

---

## 1. Seçeneklerin Karşılaştırması

| Sağlayıcı / Yöntem | Yıllık Maliyet | Bireysel (TR) Uygunluğu | Anahtar Depolama | Yayıncı Adı (`Subject`) | SmartScreen & Notlar |
|---|---|---|---|---|---|
| **SignPath Foundation** *(OSS Programı)* | **Ücretsiz** | Evet (açık kaynak proje) | SignPath Bulut HSM | `SignPath Foundation` | OSI lisansı (`MIT OR Apache-2.0` uyumlu), herkese açık GitHub deposu ve sürüm/bakım geçmişi şartı; CI'da tam otomatik imzalama. |
| **Certum Open Source** *(SimplySign Bulut HSM)* | **~€49 – \$58 / yıl** *(veya kart+okuyucu seti ~€69)* | **Evet** (dünya geneli açık) | Bulut HSM (SimplySign) | `Open Source Developer, <Ad Soyad>` | Fiziksel USB/kargo gerektirmez; resmi kimlik + adres belgesi + açık kaynak repo kanıtı gerekir; yalnızca ticari olmayan OSS projelerde kullanılabilir. |
| **Microsoft Azure Artifact Signing** *(eski Trusted Signing)* | **\$9.99 / ay** (~ \$120 / yıl, 5.000 imza/ay) | **Hayır** *(yalnızca ABD & Kanada bireysellere açık)* | Azure Bulut HSM | Doğrulanmış ad / şirket | Türkiye'den bireysel başvuru desteklenmez (desteklenen bölgelerde kayıtlı şirket şartı vardır). |
| **Ticari OV Sertifikası** *(SSL.com / Sectigo / DigiCert)* | **~\$129 – \$439 / yıl** | Çoğunlukla şirket doğrulamalı | FIPS 140-2 L2+ USB Token veya Bulut HSM (eSigner/KeyLocker) | Kurum / geliştirici adı | Haziran 2023 CA/B Forum kuralı gereği PFX dosya dışa aktarımı yoktur; donanım token kargosu veya bulut HSM ek ücreti vardır. |
| **Ticari EV Sertifikası** *(SSL.com / Sectigo / DigiCert)* | **~\$290 – \$644+ / yıl** | Şirket doğrulamalı | FIPS 140-2 L2+ USB Token veya Bulut HSM | Kurum adı (EV) | **Önemli:** Microsoft, yeni imzalı ikililer için EV'de anında SmartScreen bypass garantisini kaldırdı. Yalnızca kernel sürücüsü (WHQL/HDC) için şarttır. |
| **GitHub Artifact Attestations** *(Sigstore)* | **Ücretsiz** | Evet | Keyless OIDC | — | Build provenance (SLSA) kanıtı sunar ancak Windows Authenticode / SmartScreen yerine geçmez; `SHA256SUMS.txt` doğrulamasını tamamlar. |

---

## 2. SmartScreen ve Kernel Sürücü Gerçeği

1. **Omvix Hub user-mode bir uygulamadır:** Kernel sürücüsü gereksinimi **yoktur** (isteğe bağlı `PawnIO` sürücüsü zaten `namazso.eu` tarafından imzalanmış olarak ayrı kurulur). Bu nedenle pahalı bir **EV** sertifikasına teknik olarak ihtiyaç yoktur.
2. **SmartScreen İtibarı:** Microsoft Defender SmartScreen uyarıları artık sertifika türüne bakılmaksızın büyük ölçüde **indirme hacmi ve telemetri geçmişine** bağlıdır. Geçerli bir Authenticode imzası "Bilinmeyen Yayıncı" temel güvensizliğini ortadan kaldırır ve sertifikanın indirme itibarı sürüm güncellemelerinde korunur (unsigned `.exe` dosyalarında itibar her sürümde sıfırlanır).
3. **Zaman Damgası (Timestamping):** `signtool.exe` ile RFC 3161 zaman damgası (`/tr ... /td sha256`) basıldığında, sertifika süresi dolsa bile imzalanan ikili geçerliliğini korur.

---

## 3. Omvix Hub İçin Önerilen Karar Yolu

- **1. Tercih (Maliyetsiz / CI Otomatik):** **SignPath Foundation OSS** başvurusu  
  - Omvix Hub `MIT OR Apache-2.0` lisansına ve herkese açık `Borab55/hp-omen-hub-lite` deposuna sahip olduğu için uygundur.
  - Başvuru kabul edildiğinde release workflow imzalamayı bulut HSM üzerinden otomatikleştirir; yerel sertifika/token yönetimi gerekmez.
- **2. Tercih (Düşük Maliyetli / Kişisel Yayıncı Adı):** **Certum Open Source Code Signing (SimplySign Bulut)** (~€49 / \$58 yıllık)  
  - SignPath beklemeden kendi adına (`Open Source Developer`) imzalamak istersen Türkiye'den bireysel olarak başvurabileceğin en ekonomik bulut HSM seçeneğidir.

---

## 4. Güncelleyici Entegrasyonu (Sertifika Alındığında Yapılacaklar)

Omvix Hub'ın yerleşik güncelleyicisi (`v0.5.0`) varsayılan olarak **imzasız kurulumu reddeder** (`allow_unsigned_sha256_updates = false`). Sertifika temin edildiğinde yalnızca şu iki adım gerekir:

1. [`crates/core/src/updater.rs`](../crates/core/src/updater.rs) içindeki `PINNED_SIGNER_SUBJECT` sabitini sertifikanın kesin `Subject` değeriyle doldurmak (ör. `Some("CN=SignPath Foundation, ...")` veya `Some("CN=Open Source Developer, ...")`).
2. GitHub Releases derleme iş akışında `omvix-hub.exe` ve `omvix-cli.exe` dosyalarını imzaladıktan sonra `SHA256SUMS.txt` özetlerini üretip release varlıklarına eklemek (imzalama PE dosyasının hash'ini değiştirdiği için `SHA256SUMS.txt` her zaman imzalamadan **sonra** oluşturulmalıdır).
