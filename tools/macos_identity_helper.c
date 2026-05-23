#include <CoreFoundation/CoreFoundation.h>
#include <Security/Security.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static const char *kService = "com.opengpu.device.identity";

static char *hex_encode(const uint8_t *bytes, size_t length) {
    char *output = calloc(length * 2 + 1, sizeof(char));
    if (output == NULL) {
        return NULL;
    }

    static const char *digits = "0123456789abcdef";
    for (size_t index = 0; index < length; ++index) {
        output[index * 2] = digits[(bytes[index] >> 4) & 0x0f];
        output[index * 2 + 1] = digits[bytes[index] & 0x0f];
    }
    output[length * 2] = '\0';
    return output;
}

static uint8_t hex_digit(char value) {
    if (value >= '0' && value <= '9') {
        return (uint8_t)(value - '0');
    }
    if (value >= 'a' && value <= 'f') {
        return (uint8_t)(value - 'a' + 10);
    }
    if (value >= 'A' && value <= 'F') {
        return (uint8_t)(value - 'A' + 10);
    }
    return 0xff;
}

static CFDataRef data_from_hex(const char *hex) {
    if (hex == NULL) {
        return NULL;
    }

    size_t len = strlen(hex);
    if ((len % 2) != 0) {
        return NULL;
    }

    size_t out_len = len / 2;
    uint8_t *buffer = calloc(out_len, sizeof(uint8_t));
    if (buffer == NULL) {
        return NULL;
    }

    for (size_t i = 0; i < out_len; ++i) {
        uint8_t high = hex_digit(hex[i * 2]);
        uint8_t low = hex_digit(hex[i * 2 + 1]);
        if (high == 0xff || low == 0xff) {
            free(buffer);
            return NULL;
        }
        buffer[i] = (uint8_t)((high << 4) | low);
    }

    CFDataRef data = CFDataCreate(kCFAllocatorDefault, buffer, (CFIndex)out_len);
    free(buffer);
    return data;
}

static char *string_copy_utf8(CFStringRef value) {
    if (value == NULL) {
        return NULL;
    }

    CFIndex length = CFStringGetLength(value);
    CFIndex max_size = CFStringGetMaximumSizeForEncoding(length, kCFStringEncodingUTF8) + 1;
    char *buffer = calloc((size_t)max_size, sizeof(char));
    if (buffer == NULL) {
        return NULL;
    }

    if (!CFStringGetCString(value, buffer, max_size, kCFStringEncodingUTF8)) {
        free(buffer);
        return NULL;
    }

    return buffer;
}

static void print_cf_error(CFErrorRef error) {
    if (error == NULL) {
        return;
    }

    CFStringRef description = CFErrorCopyDescription(error);
    char *message = string_copy_utf8(description);
    if (message != NULL) {
        fprintf(stderr, "%s\n", message);
        free(message);
    }
    if (description != NULL) {
        CFRelease(description);
    }
}

static CFDataRef application_label_from_key(SecKeyRef key) {
    CFDictionaryRef attributes = SecKeyCopyAttributes(key);
    if (attributes == NULL) {
        return NULL;
    }

    CFDataRef label = (CFDataRef)CFDictionaryGetValue(attributes, kSecAttrApplicationLabel);
    if (label != NULL) {
        CFRetain(label);
    }
    CFRelease(attributes);
    return label;
}

static CFDataRef public_key_data_from_key(SecKeyRef key, CFErrorRef *error) {
    SecKeyRef public_key = SecKeyCopyPublicKey(key);
    if (public_key == NULL) {
        return NULL;
    }

    CFDataRef public_data = SecKeyCopyExternalRepresentation(public_key, error);
    CFRelease(public_key);
    return public_data;
}

static SecKeyRef load_private_key(const char *label_hex, bool allow_create, bool *created, CFErrorRef *error) {
    if (label_hex != NULL && label_hex[0] != '\0') {
        CFDataRef label_data = data_from_hex(label_hex);
        if (label_data == NULL) {
            if (created != NULL) {
                *created = false;
            }
            return NULL;
        }

        const void *query_keys[] = {
            kSecClass,
            kSecAttrApplicationLabel,
            kSecReturnRef,
            kSecMatchLimit,
        };
        const void *query_values[] = {
            kSecClassKey,
            label_data,
            kCFBooleanTrue,
            kSecMatchLimitOne,
        };

        CFDictionaryRef query = CFDictionaryCreate(
            kCFAllocatorDefault,
            query_keys,
            query_values,
            4,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks
        );
        CFRelease(label_data);
        if (query == NULL) {
            if (created != NULL) {
                *created = false;
            }
            return NULL;
        }

        CFTypeRef item = NULL;
        OSStatus status = SecItemCopyMatching(query, &item);
        CFRelease(query);
        if (status == errSecSuccess && item != NULL) {
            if (created != NULL) {
                *created = false;
            }
            return (SecKeyRef)item;
        }

        if (status != errSecItemNotFound) {
            fprintf(stderr, "private key lookup failed with status %d\n", (int)status);
            if (error != NULL) {
                *error = CFErrorCreate(kCFAllocatorDefault, kCFErrorDomainOSStatus, status, NULL);
            }
            if (created != NULL) {
                *created = false;
            }
            return NULL;
        }
    }

    if (!allow_create) {
        if (created != NULL) {
            *created = false;
        }
        if (error != NULL) {
            *error = CFErrorCreate(kCFAllocatorDefault, kCFErrorDomainOSStatus, errSecItemNotFound, NULL);
        }
        return NULL;
    }

    CFNumberRef key_size = NULL;
    int key_bits = 256;
    key_size = CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &key_bits);
    if (key_size == NULL) {
        if (created != NULL) {
            *created = false;
        }
        return NULL;
    }

    CFStringRef label = CFStringCreateWithCString(kCFAllocatorDefault, kService, kCFStringEncodingUTF8);
    if (label == NULL) {
        CFRelease(key_size);
        if (created != NULL) {
            *created = false;
        }
        return NULL;
    }

    const void *attrs_keys[] = {
        kSecAttrKeyType,
        kSecAttrKeySizeInBits,
        kSecAttrIsPermanent,
        kSecAttrTokenID,
        kSecAttrLabel,
        kSecAttrAccessControl,
    };

    CFErrorRef access_error = NULL;
    SecAccessControlRef access_control = SecAccessControlCreateWithFlags(
        kCFAllocatorDefault,
        kSecAttrAccessibleWhenUnlocked,
        kSecAccessControlPrivateKeyUsage,
        &access_error
    );
    if (access_control == NULL) {
        print_cf_error(access_error);
        if (access_error != NULL) {
            CFRelease(access_error);
        }
        CFRelease(key_size);
        CFRelease(label);
        if (created != NULL) {
            *created = false;
        }
        return NULL;
    }

    const void *attrs_values[] = {
        kSecAttrKeyTypeEC,
        key_size,
        kCFBooleanTrue,
        kSecAttrTokenIDSecureEnclave,
        label,
        access_control,
    };

    CFDictionaryRef attrs = CFDictionaryCreate(
        kCFAllocatorDefault,
        attrs_keys,
        attrs_values,
        5,
        &kCFTypeDictionaryKeyCallBacks,
        &kCFTypeDictionaryValueCallBacks
    );
    CFRelease(key_size);
    CFRelease(label);
    CFRelease(access_control);
    if (attrs == NULL) {
        if (created != NULL) {
            *created = false;
        }
        return NULL;
    }

    SecKeyRef private_key = NULL;
    SecKeyRef public_key = NULL;
    OSStatus status = SecKeyGeneratePair(attrs, &public_key, &private_key);
    CFRelease(attrs);
    if (public_key != NULL) {
        CFRelease(public_key);
    }
    if (status != errSecSuccess || private_key == NULL) {
        fprintf(stderr, "key creation failed with status %d\n", (int)status);
        if (created != NULL) {
            *created = false;
        }
        return NULL;
    }

    if (created != NULL) {
        *created = true;
    }
    return private_key;
}

static CFDataRef read_stdin(void) {
    size_t capacity = 4096;
    size_t length = 0;
    uint8_t *buffer = malloc(capacity);
    if (buffer == NULL) {
        return NULL;
    }

    while (true) {
        if (length == capacity) {
            capacity *= 2;
            uint8_t *next = realloc(buffer, capacity);
            if (next == NULL) {
                free(buffer);
                return NULL;
            }
            buffer = next;
        }

        size_t read = fread(buffer + length, 1, capacity - length, stdin);
        length += read;
        if (read == 0) {
            break;
        }
    }

    CFDataRef data = CFDataCreate(kCFAllocatorDefault, buffer, (CFIndex)length);
    free(buffer);
    return data;
}

static int ensure_identity(const char *label_hex) {
    CFErrorRef error = NULL;
    bool created = false;
    SecKeyRef private_key = load_private_key(label_hex, true, &created, &error);
    if (private_key == NULL) {
        print_cf_error(error);
        if (error != NULL) {
            CFRelease(error);
        }
        return 1;
    }

    CFErrorRef public_error = NULL;
    CFDataRef public_data = public_key_data_from_key(private_key, &public_error);
    if (public_data == NULL) {
        print_cf_error(public_error);
        if (public_error != NULL) {
            CFRelease(public_error);
        }
        CFRelease(private_key);
        return 1;
    }

    CFDataRef app_label = application_label_from_key(private_key);
    if (app_label == NULL) {
        fprintf(stderr, "failed to read application label\n");
        CFRelease(public_data);
        CFRelease(private_key);
        return 1;
    }

    const UInt8 *public_bytes = CFDataGetBytePtr(public_data);
    CFIndex public_length = CFDataGetLength(public_data);
    char *public_hex = hex_encode(public_bytes, (size_t)public_length);
    if (public_hex == NULL) {
        fprintf(stderr, "failed to encode public key\n");
        CFRelease(app_label);
        CFRelease(public_data);
        CFRelease(private_key);
        return 1;
    }

    const UInt8 *label_bytes = CFDataGetBytePtr(app_label);
    CFIndex label_length = CFDataGetLength(app_label);
    char *label_hex_output = hex_encode(label_bytes, (size_t)label_length);
    if (label_hex_output == NULL) {
        fprintf(stderr, "failed to encode application label\n");
        free(public_hex);
        CFRelease(app_label);
        CFRelease(public_data);
        CFRelease(private_key);
        return 1;
    }

    CFStringRef fingerprint_ref = NULL;
    {
        char *fingerprint_buffer = hex_encode(public_bytes, (size_t)public_length);
        if (fingerprint_buffer == NULL) {
            fprintf(stderr, "failed to compute fingerprint\n");
            free(public_hex);
            free(label_hex_output);
            CFRelease(app_label);
            CFRelease(public_data);
            CFRelease(private_key);
            return 1;
        }

        size_t prefix_len = strlen(fingerprint_buffer) < 16 ? strlen(fingerprint_buffer) : 16;
        char prefix[17];
        memcpy(prefix, fingerprint_buffer, prefix_len);
        prefix[prefix_len] = '\0';
        fingerprint_ref = CFStringCreateWithCString(kCFAllocatorDefault, prefix, kCFStringEncodingUTF8);
        free(fingerprint_buffer);
    }

    char *fingerprint = string_copy_utf8(fingerprint_ref);
    if (fingerprint == NULL) {
        fingerprint = strdup("error");
    }

    printf(
        "{\"public_key_hex\":\"%s\",\"fingerprint\":\"%s\",\"keychain_label_hex\":\"%s\",\"created\":%s}\n",
        public_hex,
        fingerprint,
        label_hex_output,
        created ? "true" : "false"
    );

    free(public_hex);
    free(label_hex_output);
    free(fingerprint);
    if (fingerprint_ref != NULL) {
        CFRelease(fingerprint_ref);
    }
    CFRelease(app_label);
    CFRelease(public_data);
    CFRelease(private_key);
    return 0;
}

static int sign_message(const char *label_hex) {
    if (label_hex == NULL || label_hex[0] == '\0') {
        fprintf(stderr, "missing keychain label\n");
        return 2;
    }

    CFErrorRef error = NULL;
    bool created = false;
    SecKeyRef private_key = load_private_key(label_hex, false, &created, &error);
    if (private_key == NULL) {
        print_cf_error(error);
        if (error != NULL) {
            CFRelease(error);
        }
        return 1;
    }

    CFDataRef stdin_data = read_stdin();
    if (stdin_data == NULL) {
        CFRelease(private_key);
        fprintf(stderr, "failed to read stdin\n");
        return 1;
    }

    CFDataRef signature = SecKeyCreateSignature(
        private_key,
        kSecKeyAlgorithmECDSASignatureMessageX962SHA256,
        stdin_data,
        &error
    );
    if (signature == NULL) {
        print_cf_error(error);
        if (error != NULL) {
            CFRelease(error);
        }
        CFRelease(stdin_data);
        CFRelease(private_key);
        return 1;
    }

    const UInt8 *bytes = CFDataGetBytePtr(signature);
    CFIndex length = CFDataGetLength(signature);
    char *hex = hex_encode(bytes, (size_t)length);
    if (hex == NULL) {
        fprintf(stderr, "failed to encode signature\n");
        CFRelease(signature);
        CFRelease(stdin_data);
        CFRelease(private_key);
        return 1;
    }

    printf("%s\n", hex);
    free(hex);
    CFRelease(signature);
    CFRelease(stdin_data);
    CFRelease(private_key);
    return 0;
}

static int verify_message(const char *public_key_hex, const char *signature_hex) {
    CFErrorRef error = NULL;
    SecKeyRef public_key = NULL;
    CFDataRef public_data = data_from_hex(public_key_hex);
    if (public_data == NULL) {
        fprintf(stderr, "invalid public key hex\n");
        return 1;
    }

    const void *attrs_keys[] = {
        kSecAttrKeyType,
        kSecAttrKeyClass,
        kSecAttrKeySizeInBits,
    };
    const void *attrs_values[] = {
        kSecAttrKeyTypeEC,
        kSecAttrKeyClassPublic,
        CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &(int){256}),
    };

    CFDictionaryRef attrs = CFDictionaryCreate(
        kCFAllocatorDefault,
        attrs_keys,
        attrs_values,
        3,
        &kCFTypeDictionaryKeyCallBacks,
        &kCFTypeDictionaryValueCallBacks
    );
    CFRelease((CFNumberRef)attrs_values[2]);
    if (attrs == NULL) {
        CFRelease(public_data);
        fprintf(stderr, "failed to build public key attributes\n");
        return 1;
    }

    public_key = SecKeyCreateWithData(public_data, attrs, &error);
    CFRelease(attrs);
    CFRelease(public_data);
    if (public_key == NULL) {
        print_cf_error(error);
        if (error != NULL) {
            CFRelease(error);
        }
        return 1;
    }

    CFDataRef signature = data_from_hex(signature_hex);
    if (signature == NULL) {
        CFRelease(public_key);
        fprintf(stderr, "invalid signature hex\n");
        return 1;
    }

    CFDataRef stdin_data = read_stdin();
    if (stdin_data == NULL) {
        CFRelease(signature);
        CFRelease(public_key);
        fprintf(stderr, "failed to read stdin\n");
        return 1;
    }

    Boolean ok = SecKeyVerifySignature(
        public_key,
        kSecKeyAlgorithmECDSASignatureMessageX962SHA256,
        stdin_data,
        signature,
        &error
    );
    if (!ok) {
        print_cf_error(error);
        if (error != NULL) {
            CFRelease(error);
        }
        CFRelease(signature);
        CFRelease(stdin_data);
        CFRelease(public_key);
        printf("fail\n");
        return 1;
    }

    printf("ok\n");
    CFRelease(signature);
    CFRelease(stdin_data);
    CFRelease(public_key);
    return 0;
}

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: macos_identity_helper ensure|sign|verify\n");
        return 2;
    }

    if (strcmp(argv[1], "ensure") == 0) {
        const char *label_hex = argc >= 3 ? argv[2] : NULL;
        return ensure_identity(label_hex);
    }
    if (strcmp(argv[1], "sign") == 0) {
        const char *label_hex = argc >= 3 ? argv[2] : NULL;
        return sign_message(label_hex);
    }
    if (strcmp(argv[1], "verify") == 0) {
        if (argc < 4) {
            fprintf(stderr, "usage: macos_identity_helper verify <public_key_hex> <signature_hex>\n");
            return 2;
        }
        return verify_message(argv[2], argv[3]);
    }

    fprintf(stderr, "unknown command: %s\n", argv[1]);
    return 2;
}
