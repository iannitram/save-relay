#include <jni.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "save_relay.h"

static JavaVM *g_vm = NULL;
static jobject g_host = NULL;
static jmethodID g_credential_get = NULL;
static jmethodID g_credential_set = NULL;
static jmethodID g_credential_delete = NULL;
static jmethodID g_http = NULL;

static JNIEnv *get_env(int *attached) {
  JNIEnv *env = NULL;
  *attached = 0;
  if (g_vm == NULL) {
    return NULL;
  }
  jint status = (*g_vm)->GetEnv(g_vm, (void **)&env, JNI_VERSION_1_6);
  if (status == JNI_EDETACHED) {
    if ((*g_vm)->AttachCurrentThread(g_vm, (void **)&env, NULL) != JNI_OK) {
      return NULL;
    }
    *attached = 1;
  } else if (status != JNI_OK) {
    return NULL;
  }
  return env;
}

static void release_env(int attached) {
  if (attached && g_vm != NULL) {
    (*g_vm)->DetachCurrentThread(g_vm);
  }
}

static int host_credential_get(void *user_data, const char *account,
                               uint8_t *buf, size_t cap, size_t *len) {
  (void)user_data;
  int attached = 0;
  int result = 2;
  JNIEnv *env = get_env(&attached);
  if (env == NULL || g_host == NULL || buf == NULL || len == NULL) {
    release_env(attached);
    return 2;
  }
  jstring jaccount = (*env)->NewStringUTF(env, account);
  if (jaccount != NULL) {
    jbyteArray secret = (jbyteArray)(*env)->CallObjectMethod(
        env, g_host, g_credential_get, jaccount);
    if ((*env)->ExceptionCheck(env)) {
      (*env)->ExceptionClear(env);
    } else if (secret == NULL) {
      result = 1;
    } else {
      jsize size = (*env)->GetArrayLength(env, secret);
      if (size >= 0 && (size_t)size <= cap) {
        (*env)->GetByteArrayRegion(env, secret, 0, size, (jbyte *)buf);
        if ((*env)->ExceptionCheck(env)) {
          (*env)->ExceptionClear(env);
        } else {
          *len = (size_t)size;
          result = 0;
        }
      }
      (*env)->DeleteLocalRef(env, secret);
    }
    (*env)->DeleteLocalRef(env, jaccount);
  }
  release_env(attached);
  return result;
}

static int host_credential_set(void *user_data, const char *account,
                               const char *secret) {
  (void)user_data;
  int attached = 0;
  int result = 2;
  JNIEnv *env = get_env(&attached);
  if (env == NULL || g_host == NULL) {
    release_env(attached);
    return 2;
  }
  jstring jaccount = (*env)->NewStringUTF(env, account);
  jstring jsecret = (*env)->NewStringUTF(env, secret);
  if (jaccount != NULL && jsecret != NULL) {
    jboolean ok = (*env)->CallBooleanMethod(env, g_host, g_credential_set,
                                            jaccount, jsecret);
    if ((*env)->ExceptionCheck(env)) {
      (*env)->ExceptionClear(env);
    } else {
      result = ok ? 0 : 2;
    }
  }
  if (jaccount != NULL) {
    (*env)->DeleteLocalRef(env, jaccount);
  }
  if (jsecret != NULL) {
    (*env)->DeleteLocalRef(env, jsecret);
  }
  release_env(attached);
  return result;
}

static int host_credential_delete(void *user_data, const char *account) {
  (void)user_data;
  int attached = 0;
  int result = 2;
  JNIEnv *env = get_env(&attached);
  if (env == NULL || g_host == NULL) {
    release_env(attached);
    return 2;
  }
  jstring jaccount = (*env)->NewStringUTF(env, account);
  if (jaccount != NULL) {
    jboolean ok = (*env)->CallBooleanMethod(env, g_host, g_credential_delete,
                                            jaccount);
    if ((*env)->ExceptionCheck(env)) {
      (*env)->ExceptionClear(env);
    } else {
      result = ok ? 0 : 2;
    }
    (*env)->DeleteLocalRef(env, jaccount);
  }
  release_env(attached);
  return result;
}

static int host_http(void *user_data, const SaveRelayHttpRequest *request,
                     SaveRelayHttpSink *sink) {
  (void)user_data;
  if (request == NULL || sink == NULL) {
    return 2;
  }
  int attached = 0;
  int result = 1;
  JNIEnv *env = get_env(&attached);
  if (env == NULL || g_host == NULL) {
    release_env(attached);
    return 1;
  }

  jstring method = (*env)->NewStringUTF(env, request->method);
  jstring url = (*env)->NewStringUTF(env, request->url);
  jstring headers = request->headers == NULL
                        ? NULL
                        : (*env)->NewStringUTF(env, request->headers);
  jbyteArray body = (*env)->NewByteArray(env, (jsize)request->body_len);
  if (body != NULL && request->body_len > 0 && request->body != NULL) {
    (*env)->SetByteArrayRegion(env, body, 0, (jsize)request->body_len,
                               (const jbyte *)request->body);
  }

  if (method != NULL && url != NULL && body != NULL &&
      !(*env)->ExceptionCheck(env)) {
    jbyteArray answer = (jbyteArray)(*env)->CallObjectMethod(
        env, g_host, g_http, method, url, headers, body,
        (jlong)request->max_response_bytes);
    if ((*env)->ExceptionCheck(env)) {
      (*env)->ExceptionClear(env);
    } else if (answer != NULL) {
      jsize size = (*env)->GetArrayLength(env, answer);
      if (size >= 8) {
        jbyte *bytes = (*env)->GetByteArrayElements(env, answer, NULL);
        if (bytes != NULL) {
          const uint8_t *wire = (const uint8_t *)bytes;
          uint32_t status = ((uint32_t)wire[0] << 24) |
                            ((uint32_t)wire[1] << 16) |
                            ((uint32_t)wire[2] << 8) | (uint32_t)wire[3];
          uint32_t header_len = ((uint32_t)wire[4] << 24) |
                                ((uint32_t)wire[5] << 16) |
                                ((uint32_t)wire[6] << 8) | (uint32_t)wire[7];
          if ((size_t)size >= (size_t)8 + (size_t)header_len) {
            char *header_text = (char *)malloc((size_t)header_len + 1);
            if (header_text != NULL) {
              if (header_len > 0) {
                memcpy(header_text, wire + 8, (size_t)header_len);
              }
              header_text[header_len] = '\0';
              const uint8_t *body_pointer = wire + 8 + header_len;
              size_t body_len = (size_t)size - 8 - (size_t)header_len;
              if (save_relay_http_respond(
                      sink, (uint16_t)status,
                      header_len > 0 ? header_text : NULL,
                      body_len > 0 ? body_pointer : NULL, body_len) == 0) {
                result = 0;
              }
              free(header_text);
            }
          }
          (*env)->ReleaseByteArrayElements(env, answer, bytes, JNI_ABORT);
        }
      }
      (*env)->DeleteLocalRef(env, answer);
    }
  }

  if (method != NULL) {
    (*env)->DeleteLocalRef(env, method);
  }
  if (url != NULL) {
    (*env)->DeleteLocalRef(env, url);
  }
  if (headers != NULL) {
    (*env)->DeleteLocalRef(env, headers);
  }
  if (body != NULL) {
    (*env)->DeleteLocalRef(env, body);
  }
  release_env(attached);
  return result;
}

JNIEXPORT jint JNICALL JNI_OnLoad(JavaVM *vm, void *reserved) {
  (void)reserved;
  g_vm = vm;
  return JNI_VERSION_1_6;
}

#define ENTER_STRING(value) \
  ((value) == NULL ? NULL : (*env)->GetStringUTFChars(env, (value), NULL))

#define LEAVE_STRING(value, chars)                                      \
  do {                                                                  \
    if ((value) != NULL && (chars) != NULL) {                           \
      (*env)->ReleaseStringUTFChars(env, (value), (chars));             \
    }                                                                   \
  } while (0)

JNIEXPORT jlong JNICALL Java_org_saverelay_SaveRelay_nativeInit(
    JNIEnv *env, jobject thiz, jstring home, jstring data, jstring config,
    jstring documents, jobject host) {
  (void)thiz;
  const char *home_text = ENTER_STRING(home);
  const char *data_text = ENTER_STRING(data);
  const char *config_text = ENTER_STRING(config);
  const char *documents_text = ENTER_STRING(documents);

  if (g_host != NULL) {
    (*env)->DeleteGlobalRef(env, g_host);
    g_host = NULL;
  }
  g_credential_get = NULL;
  g_credential_set = NULL;
  g_credential_delete = NULL;
  g_http = NULL;

  if (host != NULL) {
    jclass cls = (*env)->GetObjectClass(env, host);
    if (cls != NULL) {
      g_credential_get = (*env)->GetMethodID(
          env, cls, "credentialGet", "(Ljava/lang/String;)[B");
      if (g_credential_get == NULL) {
        (*env)->ExceptionClear(env);
      }
      g_credential_set = (*env)->GetMethodID(
          env, cls, "credentialSet",
          "(Ljava/lang/String;Ljava/lang/String;)Z");
      if (g_credential_set == NULL) {
        (*env)->ExceptionClear(env);
      }
      g_credential_delete = (*env)->GetMethodID(
          env, cls, "credentialDelete", "(Ljava/lang/String;)Z");
      if (g_credential_delete == NULL) {
        (*env)->ExceptionClear(env);
      }
      g_http = (*env)->GetMethodID(
          env, cls, "http", "(Ljava/lang/String;Ljava/lang/String;"
                           "Ljava/lang/String;[BJ)[B");
      if (g_http == NULL) {
        (*env)->ExceptionClear(env);
      }
      (*env)->DeleteLocalRef(env, cls);
    }
    if (g_credential_get != NULL && g_credential_set != NULL &&
        g_credential_delete != NULL && g_http != NULL) {
      g_host = (*env)->NewGlobalRef(env, host);
    }
  }

  char *line;
  if (g_host != NULL && home_text != NULL && data_text != NULL &&
      config_text != NULL) {
    SaveRelayHost native = {g_host, home_text, data_text, config_text,
                            documents_text, host_credential_get,
                            host_credential_set, host_credential_delete,
                            host_http};
    line = save_relay_init(&native);
  } else {
    line = save_relay_init(NULL);
  }

  LEAVE_STRING(home, home_text);
  LEAVE_STRING(data, data_text);
  LEAVE_STRING(config, config_text);
  LEAVE_STRING(documents, documents_text);
  return (jlong)(intptr_t)line;
}

JNIEXPORT jstring JNICALL Java_org_saverelay_SaveRelay_nativeRead(
    JNIEnv *env, jobject thiz, jlong pointer) {
  (void)thiz;
  const char *line = (const char *)(intptr_t)pointer;
  if (line == NULL) {
    return NULL;
  }
  return (*env)->NewStringUTF(env, line);
}

JNIEXPORT void JNICALL Java_org_saverelay_SaveRelay_nativeFree(
    JNIEnv *env, jobject thiz, jlong pointer) {
  (void)env;
  (void)thiz;
  save_relay_free((char *)(intptr_t)pointer);
}

JNIEXPORT void JNICALL Java_org_saverelay_SaveRelay_nativeShutdown(
    JNIEnv *env, jobject thiz) {
  (void)thiz;
  if (g_host != NULL) {
    (*env)->DeleteGlobalRef(env, g_host);
    g_host = NULL;
  }
}

JNIEXPORT jlong JNICALL Java_org_saverelay_SaveRelay_nativePull(
    JNIEnv *env, jobject thiz, jstring manifest, jstring saves,
    jstring resolve) {
  (void)thiz;
  const char *manifest_text = ENTER_STRING(manifest);
  const char *saves_text = ENTER_STRING(saves);
  const char *resolve_text = ENTER_STRING(resolve);
  char *line = save_relay_pull(manifest_text, saves_text, resolve_text);
  LEAVE_STRING(manifest, manifest_text);
  LEAVE_STRING(saves, saves_text);
  LEAVE_STRING(resolve, resolve_text);
  return (jlong)(intptr_t)line;
}

JNIEXPORT jlong JNICALL Java_org_saverelay_SaveRelay_nativePush(
    JNIEnv *env, jobject thiz, jstring manifest, jstring saves,
    jstring resolve) {
  (void)thiz;
  const char *manifest_text = ENTER_STRING(manifest);
  const char *saves_text = ENTER_STRING(saves);
  const char *resolve_text = ENTER_STRING(resolve);
  char *line = save_relay_push(manifest_text, saves_text, resolve_text);
  LEAVE_STRING(manifest, manifest_text);
  LEAVE_STRING(saves, saves_text);
  LEAVE_STRING(resolve, resolve_text);
  return (jlong)(intptr_t)line;
}

JNIEXPORT jlong JNICALL Java_org_saverelay_SaveRelay_nativeStatus(
    JNIEnv *env, jobject thiz, jstring manifest, jstring saves) {
  (void)thiz;
  const char *manifest_text = ENTER_STRING(manifest);
  const char *saves_text = ENTER_STRING(saves);
  char *line = save_relay_status(manifest_text, saves_text);
  LEAVE_STRING(manifest, manifest_text);
  LEAVE_STRING(saves, saves_text);
  return (jlong)(intptr_t)line;
}

JNIEXPORT jlong JNICALL Java_org_saverelay_SaveRelay_nativeSetup(
    JNIEnv *env, jobject thiz, jstring server, jstring user, jstring password,
    jstring device, jstring root) {
  (void)thiz;
  const char *server_text = ENTER_STRING(server);
  const char *user_text = ENTER_STRING(user);
  const char *password_text = ENTER_STRING(password);
  const char *device_text = ENTER_STRING(device);
  const char *root_text = ENTER_STRING(root);
  char *line = save_relay_setup(server_text, user_text, password_text,
                                device_text, root_text);
  LEAVE_STRING(server, server_text);
  LEAVE_STRING(user, user_text);
  LEAVE_STRING(password, password_text);
  LEAVE_STRING(device, device_text);
  LEAVE_STRING(root, root_text);
  return (jlong)(intptr_t)line;
}

JNIEXPORT jlong JNICALL Java_org_saverelay_SaveRelay_nativeSignInStart(
    JNIEnv *env, jobject thiz, jstring service, jstring redirect,
    jstring device, jstring root) {
  (void)thiz;
  const char *service_text = ENTER_STRING(service);
  const char *redirect_text = ENTER_STRING(redirect);
  const char *device_text = ENTER_STRING(device);
  const char *root_text = ENTER_STRING(root);
  char *line = save_relay_sign_in_start(service_text, redirect_text,
                                        device_text, root_text);
  LEAVE_STRING(service, service_text);
  LEAVE_STRING(redirect, redirect_text);
  LEAVE_STRING(device, device_text);
  LEAVE_STRING(root, root_text);
  return (jlong)(intptr_t)line;
}

JNIEXPORT jlong JNICALL Java_org_saverelay_SaveRelay_nativeSignInFinish(
    JNIEnv *env, jobject thiz, jstring redirect) {
  (void)thiz;
  const char *redirect_text = ENTER_STRING(redirect);
  char *line = save_relay_sign_in_finish(redirect_text);
  LEAVE_STRING(redirect, redirect_text);
  return (jlong)(intptr_t)line;
}

JNIEXPORT jlong JNICALL Java_org_saverelay_SaveRelay_nativeGameFilesPlan(
    JNIEnv *env, jobject thiz, jstring app, jstring dest, jstring state) {
  (void)thiz;
  const char *app_text = ENTER_STRING(app);
  const char *dest_text = ENTER_STRING(dest);
  const char *state_text = ENTER_STRING(state);
  char *line = save_relay_game_files_plan(app_text, dest_text, state_text);
  LEAVE_STRING(app, app_text);
  LEAVE_STRING(dest, dest_text);
  LEAVE_STRING(state, state_text);
  return (jlong)(intptr_t)line;
}

JNIEXPORT jlong JNICALL Java_org_saverelay_SaveRelay_nativeGameFilesDownload(
    JNIEnv *env, jobject thiz, jstring app, jstring dest, jstring state) {
  (void)thiz;
  const char *app_text = ENTER_STRING(app);
  const char *dest_text = ENTER_STRING(dest);
  const char *state_text = ENTER_STRING(state);
  char *line = save_relay_game_files_download(app_text, dest_text, state_text);
  LEAVE_STRING(app, app_text);
  LEAVE_STRING(dest, dest_text);
  LEAVE_STRING(state, state_text);
  return (jlong)(intptr_t)line;
}

JNIEXPORT jlong JNICALL Java_org_saverelay_SaveRelay_nativeGameFilesProgress(
    JNIEnv *env, jobject thiz) {
  (void)env;
  (void)thiz;
  return (jlong)(intptr_t)save_relay_game_files_progress();
}

JNIEXPORT jlong JNICALL Java_org_saverelay_SaveRelay_nativeGameFilesCancel(
    JNIEnv *env, jobject thiz) {
  (void)env;
  (void)thiz;
  return (jlong)(intptr_t)save_relay_game_files_cancel();
}

JNIEXPORT jlong JNICALL Java_org_saverelay_SaveRelay_nativeGameFilesCleanup(
    JNIEnv *env, jobject thiz, jstring state) {
  (void)thiz;
  const char *state_text = ENTER_STRING(state);
  char *line = save_relay_game_files_cleanup(state_text);
  LEAVE_STRING(state, state_text);
  return (jlong)(intptr_t)line;
}
