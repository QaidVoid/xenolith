/* Somewhere to draw.
 *
 * A title writes commands for hardware that is not here, and what is here
 * instead is whatever the host has. This opens that, and nothing more: an
 * instance, a device that can draw, and the queue work goes to. What to draw
 * and how is decided elsewhere.
 *
 * Opening it can fail for reasons that are nobody's fault, a machine without a
 * graphics driver being the ordinary one, so failing is not an error here. It
 * is reported once and the rest of the runtime carries on drawing nothing,
 * which is what it did before there was any of this.
 */

#ifndef _GNU_SOURCE
#define _GNU_SOURCE /* for PTHREAD_RECURSIVE_MUTEX_INITIALIZER_NP */
#endif

#include "xenolith.h"

#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <math.h>
#include <string.h>

#ifdef XENOLITH_WITH_VULKAN

#ifdef XENOLITH_WITH_WINDOW
#ifdef XENOLITH_WITH_X11
#include <X11/Xatom.h>
#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <X11/keysym.h>
#define VK_USE_PLATFORM_XLIB_KHR
#endif
#include <unistd.h>
#endif

#include <vulkan/vulkan.h>

#ifdef XENOLITH_WITH_WINDOW
/* Says why no frame will be shown, and that the run carries on.
 *
 * A machine with no way to show a window is a limit of the host and not a fault
 * in the title, so the run does not stop for it. Saying only what is missing
 * leaves a reader unable to tell whether the program gave up, which is why the
 * second half of the sentence is there. */
static void say_no_frame_will_be_shown(const char *why) {
    fprintf(stderr,
            "xenolith: no frame will be shown, because %s; the run carries on "
            "with nothing on screen\n",
            why);
}
#endif

#include <poll.h>

#ifdef XENOLITH_WITH_WAYLAND
#include <vulkan/vulkan_wayland.h>
#include <wayland-client.h>

#include "xdg-shell-client-protocol.h"
#endif

/* What was opened, kept for as long as the program runs. */
/* Whether every fragment is to be painted the same, which is a way of asking
 * whether a shape reaches the surface at all rather than a thing to run with. */
/* Says when a device turned something down, once for each thing.
 *
 * A device that has been lost takes everything after it silently: commands are
 * still written and handed over, nothing complains, and nothing is drawn ever
 * again. That looks exactly like a title that stopped asking, which is the
 * wrong thing to go looking for. So a refusal is said out loud.
 */
static int went_wrong(VkResult said, const char *what) {
    static const char *told[8];
    static unsigned int telling;

    if (said == VK_SUCCESS) {
        return 0;
    }
    for (unsigned int at = 0; at < telling; at++) {
        if (told[at] == what) {
            return 1;
        }
    }
    if (telling < 8u) {
        told[telling++] = what;
    }
    fprintf(stderr, "xenolith: a device turned down %s, saying %d%s\n", what, (int)said,
            said == VK_ERROR_DEVICE_LOST ? ", and a lost device draws nothing ever again" : "");
    return 1;
}

static int painting_over_everything(void) {
    static int settled = -1;
    if (settled < 0) {
        settled = getenv("XENOLITH_PAINT_EVERYTHING") != NULL;
    }
    return settled;
}

/* How many sayings of what a shader reads from outside are kept in hand. A
 * title changes them between draws and a frame holds many draws.
 *
 * Enough for the busiest frame a title here draws rather than for the ordinary
 * one. Running out is not refused here, because what comes round again is a
 * saying of the same shape: the descriptors are rewritten for the draw that
 * takes one, so the oldest is reused rather than being read stale. The arenas
 * below are the ones that refuse, because what a draw reads from those cannot
 * be rewritten under a draw that already used it. */
#define XENOLITH_SAYINGS 4096

/* How many lots of recorded commands there are, so that one can be recorded
 * while the device runs the one before. Everything a lot's commands read while
 * they run is kept apart for each lot, and a lot is only recorded into again
 * once the device has finished with it. */
#define XENOLITH_LOTS 2u

static struct {
    VkInstance instance;
    VkPhysicalDevice adapter;
    VkDevice device;
    VkQueue queue;
    uint32_t family;
    int opened;
    int refused;
} drawing;

/* Which queue on a device takes drawing work, or one past the end where none of
 * them does. */
static uint32_t family_that_draws(VkPhysicalDevice adapter) {
    uint32_t count = 0;
    vkGetPhysicalDeviceQueueFamilyProperties(adapter, &count, NULL);
    if (count == 0 || count > 64) {
        return count;
    }

    VkQueueFamilyProperties families[64];
    vkGetPhysicalDeviceQueueFamilyProperties(adapter, &count, families);

    for (uint32_t at = 0; at < count; at++) {
        if ((families[at].queueFlags & VK_QUEUE_GRAPHICS_BIT) != 0) {
            return at;
        }
    }
    return count;
}

/* Whether the device finishes a rectangle, which it does where it can run the
 * shader that does. */
static int rectangles_finished_on_the_device;

/* Opens what the host has, once, and says what came of it.
 *
 * Nothing is asked of the device beyond being able to draw. A title's own
 * demands are not known here, and asking for what is not needed is a way of
 * refusing a machine that would have worked.
 */
static int open_something_to_draw_with(void) {
    if (drawing.opened || drawing.refused) {
        return drawing.opened;
    }

    VkApplicationInfo about = {0};
    about.sType = VK_STRUCTURE_TYPE_APPLICATION_INFO;
    about.pApplicationName = "xenolith";
    about.apiVersion = VK_API_VERSION_1_1;

    VkInstanceCreateInfo wanted = {0};
    wanted.sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO;
    wanted.pApplicationInfo = &about;

#ifdef XENOLITH_WITH_WINDOW
    /* What is needed to put what was drawn where it can be seen. Asked for
     * whether or not a window is opened, because whether one can be is not
     * known until the instance exists. */
    static const char *const showing[] = {
        VK_KHR_SURFACE_EXTENSION_NAME,
#ifdef XENOLITH_WITH_WAYLAND
        VK_KHR_WAYLAND_SURFACE_EXTENSION_NAME,
#endif
#ifdef XENOLITH_WITH_X11
        VK_KHR_XLIB_SURFACE_EXTENSION_NAME,
#endif
    };
    wanted.enabledExtensionCount = (uint32_t)(sizeof showing / sizeof *showing);
    wanted.ppEnabledExtensionNames = showing;

    /* Asking for a way of showing that a driver does not have stops the whole
     * instance, and with it the drawing, so what is not there is dropped rather
     * than insisted on. */
    uint32_t offered = 0;
    vkEnumerateInstanceExtensionProperties(NULL, &offered, NULL);
    VkExtensionProperties *has = offered != 0 ? calloc(offered, sizeof *has) : NULL;
    const char *asking[sizeof showing / sizeof *showing];
    uint32_t asked = 0;
    if (has != NULL && vkEnumerateInstanceExtensionProperties(NULL, &offered, has) == VK_SUCCESS) {
        for (uint32_t at = 0; at < sizeof showing / sizeof *showing; at++) {
            for (uint32_t one = 0; one < offered; one++) {
                if (strcmp(has[one].extensionName, showing[at]) == 0) {
                    asking[asked++] = showing[at];
                    break;
                }
            }
        }
        wanted.enabledExtensionCount = asked;
        wanted.ppEnabledExtensionNames = asking;
    }
    free(has);
#endif

    if (vkCreateInstance(&wanted, NULL, &drawing.instance) != VK_SUCCESS) {
        drawing.refused = 1;
        fprintf(stderr, "xenolith: nothing here draws, because no graphics driver answered\n");
        return 0;
    }

    uint32_t count = 0;
    vkEnumeratePhysicalDevices(drawing.instance, &count, NULL);
    if (count == 0 || count > 16) {
        drawing.refused = 1;
        fprintf(stderr, "xenolith: nothing here draws, because the driver offered no device\n");
        return 0;
    }

    VkPhysicalDevice adapters[16];
    vkEnumeratePhysicalDevices(drawing.instance, &count, adapters);

    for (uint32_t at = 0; at < count; at++) {
        uint32_t family = family_that_draws(adapters[at]);
        uint32_t families = 0;
        vkGetPhysicalDeviceQueueFamilyProperties(adapters[at], &families, NULL);
        if (family >= families) {
            continue;
        }

        float share = 1.0f;
        VkDeviceQueueCreateInfo queue = {0};
        queue.sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO;
        queue.queueFamilyIndex = family;
        queue.queueCount = 1;
        queue.pQueuePriorities = &share;

        /* A read past the end of what a shader was given has to come back as
         * nothing rather than as a fault.
         *
         * A shader works its own address out of a register, and a register a
         * title has not written yet holds whatever was there. That address is
         * then read, and without this the device faults and the whole context
         * is lost, taking every other program on the machine's drawing with it.
         * With it, the read comes back as nothing, which is what a shader
         * reading somewhere a title never filled should see. */
        VkPhysicalDeviceFeatures what_the_device_has = {0};
        vkGetPhysicalDeviceFeatures(adapters[at], &what_the_device_has);
        VkPhysicalDeviceFeatures safely = {0};
        safely.robustBufferAccess = VK_TRUE;
        /* What finishes a rectangle, where a device can run it. */
        safely.geometryShader = what_the_device_has.geometryShader;
        rectangles_finished_on_the_device = what_the_device_has.geometryShader == VK_TRUE;
        /* A title's pictures are kept squeezed, in a way a host squeezes them
         * too, so they are handed over as they are rather than opened out
         * first. Saying so is what lets a device be handed one. */
        safely.textureCompressionBC = VK_TRUE;
        /* Painting over every fragment with the same colour, which is asked for
         * only to find out whether a shape reaches the surface at all. What a
         * shader put out stops mattering, so a shape that is there shows and one
         * that never got that far still does not. */
        if (painting_over_everything()) {
            safely.logicOp = what_the_device_has.logicOp;
        }

        VkDeviceCreateInfo making_the_device = {0};
        making_the_device.sType = VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO;
        making_the_device.queueCreateInfoCount = 1;
        making_the_device.pQueueCreateInfos = &queue;
        making_the_device.pEnabledFeatures = &safely;

#ifdef XENOLITH_WITH_WINDOW
        static const char *const chain[1] = {VK_KHR_SWAPCHAIN_EXTENSION_NAME};
        making_the_device.enabledExtensionCount = 1;
        making_the_device.ppEnabledExtensionNames = chain;
#endif

        if (vkCreateDevice(adapters[at], &making_the_device, NULL, &drawing.device) != VK_SUCCESS) {
            continue;
        }

        VkPhysicalDeviceProperties about_it;
        vkGetPhysicalDeviceProperties(adapters[at], &about_it);

        drawing.adapter = adapters[at];
        drawing.family = family;
        vkGetDeviceQueue(drawing.device, family, 0, &drawing.queue);
        drawing.opened = 1;

        fprintf(stderr, "xenolith: drawing on %s\n", about_it.deviceName);
        return 1;
    }

    drawing.refused = 1;
    fprintf(stderr, "xenolith: nothing here draws, because no device offered to\n");
    return 0;
}

/* How wide and how tall what is drawn is.
 *
 * The size this runtime tells a title the display is, so that what comes out is
 * the size a title thinks it drew rather than a size chosen here. */
#define XENOLITH_ACROSS 1280u
#define XENOLITH_DOWN 720u

/* How wide and how tall the surface drawn into is.
 *
 * Larger than a frame, because it holds a place's samples rather than its
 * pixels: a title drawing half a frame at a time with two samples a pixel draws
 * each half as a grid of 640 by 1440, and one keeping four samples a pixel
 * draws a grid twice as wide as its rows. This is the most of either that the
 * memory beside the hardware has room for. */
#define XENOLITH_GRID_ACROSS 2560u
#define XENOLITH_GRID_DOWN 1440u

/* What the surface keeps each colour as, and how many bytes that is.
 *
 * Floats rather than bytes, because a title draws into places that keep more
 * than a fraction of the whole: it works out how bright a scene is as a
 * logarithm, which is below nought for anything dim, and kept as a byte that
 * comes back as nought, which shows the whole scene white. A place that does
 * keep a fraction has what is drawn held to one by the shader instead. */
#define XENOLITH_SURFACE_FORMAT VK_FORMAT_R16G16B16A16_SFLOAT
#define XENOLITH_SURFACE_BYTES 8u

/* The part of that surface a title actually drew its last frame into, at the
 * top left. A title that draws smaller than the display, as one rendering at
 * 640 by 480 does, fills only a corner of the surface; showing the whole
 * surface would then show its frame in a corner with the rest black. This is
 * the size the title said it copied out, and only that part is shown, scaled to
 * fill the window. Nought until a title says otherwise, and then the whole
 * surface is shown, which is what a title drawing at the display's size wants. */
static uint32_t drawn_across;
static uint32_t drawn_down;

/* Each lot's own commands, how the device says it has run them, and what it
 * has to keep until it has: the room a picture was handed over through. Pictures
 * are handed over in a lot of their own that runs just before the lot's other
 * commands, so that setting one up neither waits for the lot before nor breaks
 * into the frame being recorded. */
#define XENOLITH_HELD_FOR_A_LOT 1024u

static struct {
    VkCommandBuffer main;
    VkCommandBuffer before;
    int before_open;
    VkFence done;
    int in_flight;
    unsigned long sent_at;
    VkBuffer held[XENOLITH_HELD_FOR_A_LOT];
    VkDeviceMemory held_memory[XENOLITH_HELD_FOR_A_LOT];
    uint32_t holding;
} lots[XENOLITH_LOTS];

/* Which lot is being recorded into, or is next to be, and which was handed
 * over last. */
static uint32_t current_lot;

/* How many times the room draws take has been given out again, which says
 * whether words put there earlier are still there. */
static uint32_t room_given_out_again;
static uint32_t last_sent_lot;
static unsigned long lots_sent;

/* What is drawn into, made once and kept. */
static struct {
    VkImage image;
    VkDeviceMemory memory;
    VkBuffer readable;
    VkDeviceMemory readable_memory;
    VkCommandPool pool;
    VkCommandBuffer commands;
    /* A second pool, for the work that is not a frame.
     *
     * Setting a picture up takes commands of its own, and a device wants a pool
     * kept to one thread at a time. Taking those out of the pool a frame is
     * being recorded into is two threads in one pool, so they come from here
     * instead and the frame's pool stays the frame's alone. */
    VkCommandPool apart;
    int made;
    int refused;
    /* Whether the surface has ever been read out of, which is what decides the
     * layout it is in before a frame. A device starts an image in the layout it
     * was made with, and saying otherwise in a barrier is undefined behaviour:
     * the driver is entitled to discard what is there, and one of them refused
     * the whole command buffer for it. */
    int read_out;
} surface;

/* What each of the places a title draws into holds while another is being drawn
 * into.
 *
 * A console keeps every one of them at once, in the memory beside the hardware,
 * and a title moves between them freely: it draws a scene into one place, a
 * smaller picture of it into another, eight trees into a third, and comes back
 * to the first expecting to find what it left. There is one surface here, so
 * coming back found whatever had been drawn since, and a scene built up over
 * several turns came out as only the last of them.
 *
 * Nothing may be thrown away to make this work. The copy that ends a stretch of
 * drawing says whether the colours start again, and this title never says they
 * do. So what a place holds is put aside when a title turns away from it and
 * brought back when it returns.
 *
 * Which place a title means is the shape it says it is drawing into: how wide a
 * row is, how many readings to a place, and what each holds. Two places of the
 * same shape are deliberately not told apart, because on the hardware they are
 * the same tiles of the memory beside it and a title reusing one for two things
 * is reusing the same memory.
 */
#define XENOLITH_PLACES_DRAWN_INTO 32u

static struct {
    VkImage image;
    VkDeviceMemory memory;
    uint32_t shaped;
    int taken;
    int holds_something;
    unsigned long long when;
    uint32_t laid;
    /* How much of the surface it covers: as wide as a row of it is in samples,
     * and as far down as anything was drawn into it or copied out of it. Only
     * that much is moved when it is put aside and brought back, because the
     * surface is as big as the largest place a title draws into and most are a
     * quarter of it. Moving the whole of it every time, forty times a frame,
     * was most of what a frame cost the device. */
    uint32_t across;
    uint32_t down;
} put_aside[XENOLITH_PLACES_DRAWN_INTO];
static unsigned long long places_turned_to;

/* Which of them is being drawn into now, and the shape a title last named. */
static uint32_t drawing_into_shape;
static unsigned drawing_into_which = XENOLITH_PLACES_DRAWN_INTO;

/* How the samples of a pixel are laid out: two bits each for how many across
 * and how many down. What the surface holds and what the place for how far away
 * things are holds are each laid out the way the draws that made them were,
 * and the next draw wants its own. */
#define XENOLITH_LAID(across, down) ((across) | ((down) << 2))
static uint32_t drawing_laid = XENOLITH_LAID(1u, 1u);
static uint32_t surface_laid = XENOLITH_LAID(1u, 1u);
static uint32_t how_far_laid = XENOLITH_LAID(1u, 1u);

/* How wide the place a title has named is, in samples, and how far down what
 * the surface holds of the place on it now reaches. */
static uint32_t drawing_across = XENOLITH_GRID_ACROSS;
static uint32_t surface_down;

/* Says the surface holds something as far down as this. */
static void the_surface_reaches(uint32_t down) {
    if (down > XENOLITH_GRID_DOWN) {
        down = XENOLITH_GRID_DOWN;
    }
    if (down > surface_down) {
        surface_down = down;
    }
}

/* Says what shape of place a title is about to draw into.
 *
 * Told rather than read, because the registers naming it live in the guest's
 * memory and nothing in this file reaches that. */
void xenolith_drawing_into(uint32_t shaped, uint32_t across, uint32_t down) {
    drawing_into_shape = shaped;
    drawing_laid = XENOLITH_LAID(across, down);
    drawing_across = shaped & 0x3ffffu;
    if (drawing_across == 0 || drawing_across > XENOLITH_GRID_ACROSS) {
        drawing_across = XENOLITH_GRID_ACROSS;
    }
}

/* One lot of the commands that are not a frame at a time, because they all come
 * out of the one pool kept for them. */
static pthread_mutex_t one_lot = PTHREAD_MUTEX_INITIALIZER;

/* One thread handing work to the device at a time.
 *
 * There is one queue and three things reach it: a frame being handed over, a
 * frame being shown, and the odd lot of commands that sets a picture up. Each
 * of those is kept to one thread of its own already, but a device wants the
 * queue itself kept to one thread, and three locks over three callers is not
 * that. So they share this one, taken innermost and never held while waiting
 * for either of the others. */
static pthread_mutex_t one_queue = PTHREAD_MUTEX_INITIALIZER;

/* Which kind of memory suits what is being made, or one past the end.
 *
 * Where more than one kind will do, the one asked for first is taken, so a
 * caller that would rather have a particular sort can say so and still be given
 * something that works where there is none. */
static uint32_t memory_that_suits(uint32_t allowed, VkMemoryPropertyFlags wanted) {
    VkPhysicalDeviceMemoryProperties kinds;
    vkGetPhysicalDeviceMemoryProperties(drawing.adapter, &kinds);

    for (uint32_t at = 0; at < kinds.memoryTypeCount; at++) {
        if ((allowed & (1u << at)) == 0) {
            continue;
        }
        if ((kinds.memoryTypes[at].propertyFlags & wanted) == wanted) {
            return at;
        }
    }
    return kinds.memoryTypeCount;
}

/* The same, for memory something is going to be read back out of.
 *
 * Memory a device writes and a host only ever writes to is usually given out
 * uncached, which is right for writing and very wrong for reading: every read
 * goes the whole way to the memory. Reading a frame back out of such memory cost
 * seventy six milliseconds a time here, against about a millisecond for
 * everything else the frame did. So memory that is kept in the cache is asked
 * for first, and the uncached sort is what is taken when a device offers no
 * other. */
static uint32_t memory_to_read_back_from(uint32_t allowed) {
    uint32_t kept = memory_that_suits(allowed, VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT
                                                   | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT
                                                   | VK_MEMORY_PROPERTY_HOST_CACHED_BIT);

    VkPhysicalDeviceMemoryProperties kinds;
    vkGetPhysicalDeviceMemoryProperties(drawing.adapter, &kinds);
    if (kept < kinds.memoryTypeCount) {
        return kept;
    }

    return memory_that_suits(allowed, VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT
                                          | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT);
}

/* How much of what was drawn can wait to be handed back at once.
 *
 * Copies are read back together when what was recorded is handed over, each
 * into its own share of this, so it holds a frame's worth of them: a few dozen
 * a frame in one title, the largest a whole surface of samples. Running out
 * hands over what was recorded early rather than refusing a copy. */
#define XENOLITH_HANDED_BACK_ROOM (256ull * 1024ull * 1024ull)

/* Makes what is drawn into, and somewhere to read it back through.
 *
 * Reading it back is what makes any of this checkable without a screen. */
static int make_somewhere_to_draw(void) {
    if (surface.made || surface.refused) {
        return surface.made;
    }
    if (!open_something_to_draw_with()) {
        surface.refused = 1;
        return 0;
    }

    VkImageCreateInfo wanted = {0};
    wanted.sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO;
    wanted.imageType = VK_IMAGE_TYPE_2D;
    wanted.format = XENOLITH_SURFACE_FORMAT;
    wanted.extent.width = XENOLITH_GRID_ACROSS;
    wanted.extent.height = XENOLITH_GRID_DOWN;
    wanted.extent.depth = 1;
    wanted.mipLevels = 1;
    wanted.arrayLayers = 1;
    wanted.samples = VK_SAMPLE_COUNT_1_BIT;
    wanted.tiling = VK_IMAGE_TILING_OPTIMAL;
    /* Drawn into, read back out, and cleared before a frame, which is a
     * transfer into it and has to be asked for like the other two. */
    wanted.usage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT
                   | VK_IMAGE_USAGE_TRANSFER_DST_BIT;
    wanted.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;

    if (vkCreateImage(drawing.device, &wanted, NULL, &surface.image) != VK_SUCCESS) {
        surface.refused = 1;
        return 0;
    }

    VkMemoryRequirements needs;
    vkGetImageMemoryRequirements(drawing.device, surface.image, &needs);

    VkMemoryAllocateInfo asking = {0};
    asking.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
    asking.allocationSize = needs.size;
    asking.memoryTypeIndex =
        memory_that_suits(needs.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);

    if (vkAllocateMemory(drawing.device, &asking, NULL, &surface.memory) != VK_SUCCESS
        || vkBindImageMemory(drawing.device, surface.image, surface.memory, 0) != VK_SUCCESS) {
        surface.refused = 1;
        return 0;
    }

    VkBufferCreateInfo through = {0};
    through.sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO;
    through.size = XENOLITH_HANDED_BACK_ROOM;
    /* Written into when a frame is read back, and read out of when the frame a
     * title asked to have shown is put back where a frame is shown from. */
    through.usage = VK_BUFFER_USAGE_TRANSFER_DST_BIT | VK_BUFFER_USAGE_TRANSFER_SRC_BIT;
    through.sharingMode = VK_SHARING_MODE_EXCLUSIVE;

    if (vkCreateBuffer(drawing.device, &through, NULL, &surface.readable) != VK_SUCCESS) {
        surface.refused = 1;
        return 0;
    }

    vkGetBufferMemoryRequirements(drawing.device, surface.readable, &needs);
    asking.allocationSize = needs.size;
    asking.memoryTypeIndex = memory_to_read_back_from(needs.memoryTypeBits);

    if (vkAllocateMemory(drawing.device, &asking, NULL, &surface.readable_memory) != VK_SUCCESS
        || vkBindBufferMemory(drawing.device, surface.readable, surface.readable_memory, 0)
               != VK_SUCCESS) {
        surface.refused = 1;
        return 0;
    }

    VkCommandPoolCreateInfo holding = {0};
    holding.sType = VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO;
    holding.flags = VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT;
    holding.queueFamilyIndex = drawing.family;

    if (vkCreateCommandPool(drawing.device, &holding, NULL, &surface.pool) != VK_SUCCESS
        || vkCreateCommandPool(drawing.device, &holding, NULL, &surface.apart) != VK_SUCCESS) {
        surface.refused = 1;
        return 0;
    }

    VkCommandBufferAllocateInfo one = {0};
    one.sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO;
    one.commandPool = surface.pool;
    one.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY;
    one.commandBufferCount = 1;

    VkFenceCreateInfo telling = {0};
    telling.sType = VK_STRUCTURE_TYPE_FENCE_CREATE_INFO;
    for (uint32_t lot = 0; lot < XENOLITH_LOTS; lot++) {
        if (vkAllocateCommandBuffers(drawing.device, &one, &lots[lot].main) != VK_SUCCESS
            || vkAllocateCommandBuffers(drawing.device, &one, &lots[lot].before) != VK_SUCCESS
            || vkCreateFence(drawing.device, &telling, NULL, &lots[lot].done) != VK_SUCCESS) {
            surface.refused = 1;
            return 0;
        }
    }
    surface.commands = lots[0].main;

    surface.made = 1;
    fprintf(stderr, "xenolith: drawing into %u by %u\n", XENOLITH_GRID_ACROSS, XENOLITH_GRID_DOWN);
    return 1;
}

/* How many constants a shader is handed, and how big that is in bytes.
 *
 * The console's own number, matching what a translated shader looks for. */
#define XENOLITH_CONSTANTS 256u
/* Two more than the constants a title names: what the hardware would have done
 * to a position, as something to multiply by and something to add. */
/* Two for the scale and offset a position is brought through, two more holding
 * the two hundred and fifty six truths a shader asks after when it jumps on one
 * rather than on the predicate, eight more holding the thirty two counts a
 * shader reads to know how many times to go round a loop, and twenty four
 * saying where each of the streams a fetch can name begins. */
#define XENOLITH_BESIDE_THE_CONSTANTS 36u
#define XENOLITH_THE_STARTS (XENOLITH_ALL_THE_CONSTANTS + 12u)

/* How many the hardware holds altogether: twice what one shader names, because
 * the two kinds of shader do not share them. */
#define XENOLITH_ALL_THE_CONSTANTS (XENOLITH_CONSTANTS * 2u)
#define XENOLITH_CONSTANT_BYTES \
    ((XENOLITH_ALL_THE_CONSTANTS + XENOLITH_BESIDE_THE_CONSTANTS) * 16u)

/* How much of a title's memory a shader can go and get its vertices from.
 *
 * The first words of each draw's room say where each of a title's fetches reads
 * from, and the rest is what those fetches read. A shader is told the same
 * beside its constants, counted from the front of the frame's share, which is
 * what it fetches from, so a table two draws read can be put there once.
 *
 * It is a window rather than the whole of memory because the whole of it is
 * half a gigabyte and none of a frame's drawing touches more than a little of
 * it. A fetch reaching past the end reads
 * nothing, which is what a device does with a read past the end of a buffer it
 * was given, rather than anything this decides. */
#define XENOLITH_VERTEX_SLOTS 96u
#define XENOLITH_VERTEX_WORDS (1u * 1024u * 1024u)
#define XENOLITH_VERTEX_BYTES (XENOLITH_VERTEX_WORDS * 4u)

/* How much of each buffer a whole frame's drawing is given.
 *
 * A draw is not over when the words it reads have been written: what a title
 * asks for is recorded and only run when the frame is handed over, so every
 * draw in the frame reads its words then rather than when they were written.
 * One window shared by them all therefore hands every draw the last one's
 * vertices. Each draw takes its own room out of these instead, and the room is
 * given back at the start of the next frame.
 */
#define XENOLITH_VERTEX_ROOM (64u * 1024u * 1024u)
/* How many draws one frame's constants have room for. Each draw takes the
 * whole file, which is a little over eight kilobytes. A thousand, the number of
 * shaders a run keeps, ran out on one title's first screens and left the draws
 * past it out of the frame. */
#define XENOLITH_DRAWS_A_FRAME 4096u
#define XENOLITH_CONSTANT_ROOM (XENOLITH_CONSTANT_BYTES * XENOLITH_DRAWS_A_FRAME)

/* Where a title's shader constants live in the graphics registers.
 *
 * A title writes them there through the commands it sends, four registers to
 * each of them, and they are the greater part of what its shaders read. */
#define XENOLITH_CONSTANT_REGISTER 0x4000u

/* What a frame is drawn through, made once the first draw asks for it. */
static struct {
    VkRenderPass pass;
    VkBuffer vertices;
    VkDeviceMemory vertex_memory;
    VkDescriptorSetLayout shape;
    VkDescriptorPool holding;
    /* A ring of them rather than one.
     *
     * What a shader reads from outside is said in one of these, and a title
     * changes what it reads between draws. Saying it again in the one that is
     * already part of a frame being recorded takes that frame apart: a device
     * is told the thing it was given has changed underneath it, and every draw
     * in the frame ends up reading whatever the last one said. So each saying
     * takes the next of these and leaves the ones already spoken for alone. */
    VkDescriptorSet handed[XENOLITH_SAYINGS * XENOLITH_LOTS];
    uint32_t saying_now;
    VkBuffer constants;
    VkDeviceMemory constant_memory;
    /* The two arenas, mapped once and left mapped, and how much of each this
     * frame has handed out. What the draw being set up was given is kept
     * beside them, because that is what its descriptor set has to be told. */
    uint32_t *vertex_words;
    uint8_t *constant_bytes;
    VkDeviceSize vertex_at;
    VkDeviceSize constant_at;
    VkDeviceSize vertex_from;
    VkDeviceSize vertex_wide;
    /* What number each vertex of the next draw arrives at its shader with:
     * as many words as it draws, this far into its room, or where there are
     * none, one after another from the first. */
    uint32_t numbers_at;
    uint32_t numbers;
    uint32_t first_number;
    /* How much further away what is drawn next is taken to be, in the
     * console's terms. */
    float pushed_outright;
    float pushed_along_a_slope;
    VkDeviceSize constant_from;
    VkDeviceSize offsets_line_up_on;
    VkImageView view;
    /* Somewhere to keep how far away each place on the surface is.
     *
     * Without one, what a frame shows is whatever was drawn last rather than
     * whatever is nearest, so a scene comes out with its far surfaces painted
     * over its near ones. */
    VkImage how_far;
    VkDeviceMemory how_far_memory;
    VkImageView how_far_seen;
    VkFramebuffer target;
    VkPipelineLayout layout;
    int made;
    int refused;
} through;

/* The pictures a shader can be handed, which is how many places the hardware
 * keeps fetch constants for. */
#define XENOLITH_PICTURES 32u

/* A picture as it is laid out, and what it is laid out in.
 *
 * The console keeps a picture in an order that puts the parts a reading is
 * likely to want next to one another rather than in rows, so that reading a
 * small square of it touches as little memory as possible. A host wants rows.
 * What is kept here is enough to turn the one into the other and to know when
 * a picture already turned is the one being asked for again. */
/* How many pictures already turned are kept.
 *
 * More than the places a shader reads from, because a title moves a handful of
 * pictures between those places and the same picture comes back to a place it
 * has been in before. Keeping one for each place instead means a title using
 * three pictures across two places turns one of them again on nearly every
 * draw, which is a gigabyte a minute of the same three pictures. */
#define XENOLITH_PICTURES_KEPT 1024u

static struct {
    VkImage image;
    VkDeviceMemory memory;
    VkImageView view;
    uint32_t from;
    uint32_t across;
    uint32_t down;
    uint32_t kept;
    uint32_t held_how;
    /* What the memory this was turned from added up to when it was turned. */
    uint32_t came_to;
    uint32_t over;
    /* Which frame this was last checked against its memory in. Checking it
     * again in the same frame reads the whole picture to learn what it just
     * learned. */
    unsigned long checked_in;
    VkSampler way;
    int held;
    /* How it was kept: in squares or not, how wide a run of them is, and which
     * way round its bytes were. Together they say whether a copy out of what
     * was drawn lands in it as it stands. */
    int in_squares;
    uint32_t along;
    uint32_t order;
    /* Which copy last put what was drawn into it, so that the same copy does
     * not then forget it as written over. */
    unsigned long renewed_by;
    /* Which lot of recorded commands last put a copy into it. Until that lot
     * has run, its memory is still to be written, and what the memory adds up
     * to says nothing about the picture. */
    unsigned long renewed_in;
    /* Whether what its memory adds up to is still to be worked out, because a
     * copy put into it on the device has written that memory since. */
    int came_to_unknown;
    /* Whether a copy has written its memory in a way that could not be put
     * into it on the device. It is kept rather than let go of, because the
     * next copy into it usually can be, and turned again only if it is read
     * before then. */
    int stale;
} turned[XENOLITH_PICTURES_KEPT];

/* How many frames have been shown, which is what a span of held buttons can be
 * counted in instead of seconds. */
static unsigned long frames_shown;

/* How many copies out of what was drawn there have been. */
static unsigned long copies_counted;

/* Whether commands are being recorded that have not been handed over yet.
 *
 * Everything a title asks for between one handing over and the next goes into
 * the one lot: its frames, the copies out of them, and what those copies wipe.
 * Handing each frame over and waiting for it, as a title copies something out
 * nearly a hundred times a frame, left the device idle while the processor
 * worked and the processor idle while the device did, and between them a frame
 * of a mission took a quarter of a second. */
static int batch_open;

/* How many draws have taken a saying since the lot was begun. */
static uint32_t sayings_taken;

/* Whether anything has been recorded since the lot was last handed over that
 * changes what is drawn. A lot holding only the start of a frame is not handed
 * over, so that a title waiting between frames does not have the same empty
 * frame handed over and waited for again and again. */
static int anything_recorded;


/* How many lots have been begun, which is how a picture says which one put a
 * copy into it. */
static unsigned long batches_begun;


/* Pictures whose memory a title has written over since they were turned.
 *
 * They cannot be let go of the moment that is noticed: a frame already being
 * recorded may have been told to read one, and a device reading something let
 * go of is worse than a picture that is out of date. So they wait here until
 * what was recorded has been handed over and has run. */
#define XENOLITH_LET_GO_OF 1024u

static struct {
    VkImage image;
    VkDeviceMemory memory;
    VkImageView view;
} to_let_go_of[XENOLITH_LOTS][XENOLITH_LET_GO_OF];
static uint32_t letting_go[XENOLITH_LOTS];

static void let_go_of_what_is_finished(uint32_t lot) {
    for (uint32_t at = 0; at < letting_go[lot]; at++) {
        vkDestroyImageView(drawing.device, to_let_go_of[lot][at].view, NULL);
        vkDestroyImage(drawing.device, to_let_go_of[lot][at].image, NULL);
        vkFreeMemory(drawing.device, to_let_go_of[lot][at].memory, NULL);
    }
    letting_go[lot] = 0;
}

/* What a stretch of a title's memory adds up to.
 *
 * A title writes its own pictures as well as drawing them: the letters it puts
 * on the screen are kept in one picture that it adds to as it needs them. A
 * runtime that turns a picture once and keeps it by where it came from hands
 * back the first thing that memory ever held, so every letter added after that
 * comes out as a hole. Nothing says when a title has written there, so what it
 * holds now is added up and compared against what it held when it was turned.
 *
 * Adding up costs a read of the memory, which is a fraction of turning it
 * again: turning reads the same memory, writes as much again, and asks a
 * device for somewhere to put it.
 *
 * It is added up in four runs rather than one. Each word multiplied into the
 * one before it makes every step wait on the step before, so the whole of it
 * goes at the speed of one multiply for every four bytes however fast the
 * memory is read. Four runs that do not wait on one another go about four times
 * as fast and are put together at the end. What the number comes to is only
 * ever compared against itself, so which way round it was worked out does not
 * matter. */
static uint32_t what_the_memory_adds_up_to(const uint8_t *from, uint32_t where, uint32_t over) {
    uint32_t adds[4] = {2166136261u, 2166136261u, 2166136261u, 2166136261u};
    const uint8_t *at = from + where;
    uint32_t left = over & ~15u;

    for (uint32_t step = 0; step < left; step += 16u) {
        uint32_t four[4];
        memcpy(four, at + step, sizeof four);
        adds[0] = (adds[0] ^ four[0]) * 16777619u;
        adds[1] = (adds[1] ^ four[1]) * 16777619u;
        adds[2] = (adds[2] ^ four[2]) * 16777619u;
        adds[3] = (adds[3] ^ four[3]) * 16777619u;
    }

    uint32_t together = adds[0] ^ adds[1] ^ adds[2] ^ adds[3];
    for (uint32_t step = left; step + 4u <= over; step += 4u) {
        uint32_t word;
        memcpy(&word, at + step, sizeof word);
        together = (together ^ word) * 16777619u;
    }
    return together;
}

/* Which of the pictures already turned each place a shader reads from is
 * looking at, counting from one so that nothing means no picture. */
static uint32_t pictures[XENOLITH_PICTURES];

/* What every picture is read with, and what a place holding none is read with.
 *
 * Every place has to name something a device will accept whether a shader looks
 * at it or not, so one picture of nothing is made and the places a title has
 * not filled all name it. */
static VkSampler how_a_picture_is_read;

/* How a title says a picture is to be read past its edges, and a way of reading
 * for each pair of answers it gives.
 *
 * The console keeps this beside the picture, three bits for across and three
 * for down, so it is the title's to say and not this runtime's to choose. One
 * way of reading for everything means whatever the one is chosen to be is wrong
 * for the rest: reading a picture round again where a title said to hold its
 * last row draws the whole picture over and over across whatever it is put on.
 */
static struct {
    uint32_t held;
    VkSampler sampler;
} ways_of_reading[32];
static uint32_t ways_of_reading_now;

static VkSamplerAddressMode held_the_way(uint32_t said) {
    switch (said) {
    case 0:
        return VK_SAMPLER_ADDRESS_MODE_REPEAT;
    case 1:
        return VK_SAMPLER_ADDRESS_MODE_MIRRORED_REPEAT;
    case 2:
    case 3:
        return VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE;
    default:
        return VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_BORDER;
    }
}

/* Whether a title asks for a picture to be read by the part nearest what is
 * being drawn or by mixing the parts around it.
 *
 * Nought is the nearest part and one is the mixing. Anything else is a way this
 * has no reading of, and the mixing is what those become, since it is what
 * everything was read with before any of this was read at all. */
static VkFilter read_the_way(uint32_t said) {
    return said == 0u ? VK_FILTER_NEAREST : VK_FILTER_LINEAR;
}

/* Which parts of how a picture is read say how it is sampled, and where above
 * them the four parts a shader is handed are said, three bits each. */
#define XENOLITH_HOW_IT_IS_READ 0xfffu
#define XENOLITH_PARTS_SHIFT 12u

/* Which part of a picture a shader is handed in one of the four it reads.
 *
 * A title says this for each picture rather than the picture saying it, and
 * builds the ways round a colour can be kept out of the one the hardware keeps:
 * a picture copied out with its red and blue swapped is read with them swapped
 * back here. Read as it stands instead, a scene drawn into one picture and then
 * read from it comes out with its reds blue. */
static VkComponentSwizzle which_part_is_read(uint32_t said) {
    switch (said & 7u) {
    case 0:
        return VK_COMPONENT_SWIZZLE_R;
    case 1:
        return VK_COMPONENT_SWIZZLE_G;
    case 2:
        return VK_COMPONENT_SWIZZLE_B;
    case 3:
        return VK_COMPONENT_SWIZZLE_A;
    case 4:
        return VK_COMPONENT_SWIZZLE_ZERO;
    case 5:
        return VK_COMPONENT_SWIZZLE_ONE;
    default:
        return VK_COMPONENT_SWIZZLE_IDENTITY;
    }
}

static VkSampler a_way_of_reading(uint32_t held) {
    for (uint32_t at = 0; at < ways_of_reading_now; at++) {
        if (ways_of_reading[at].held == held) {
            return ways_of_reading[at].sampler;
        }
    }
    if (ways_of_reading_now >= 32u) {
        return how_a_picture_is_read;
    }

    VkSamplerCreateInfo how = {0};
    how.sType = VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO;
    how.magFilter = read_the_way((held >> 8) & 3u);
    how.minFilter = read_the_way((held >> 10) & 3u);
    how.mipmapMode = VK_SAMPLER_MIPMAP_MODE_LINEAR;
    how.addressModeU = held_the_way(held & 7u);
    how.addressModeV = held_the_way((held >> 4) & 7u);
    how.addressModeW = how.addressModeV;
    how.borderColor = VK_BORDER_COLOR_FLOAT_TRANSPARENT_BLACK;
    how.maxLod = VK_LOD_CLAMP_NONE;

    VkSampler made = VK_NULL_HANDLE;
    if (vkCreateSampler(drawing.device, &how, NULL, &made) != VK_SUCCESS) {
        return how_a_picture_is_read;
    }
    ways_of_reading[ways_of_reading_now].held = held;
    ways_of_reading[ways_of_reading_now].sampler = made;
    ways_of_reading_now++;
    return made;
}
static VkImage nothing_to_look_at;
static VkDeviceMemory nothing_memory;
static VkImageView nothing_seen;

/* The most bytes one square of a picture takes, which is what the widest of the
 * ways a title keeps one comes to. */
#define XENOLITH_A_SQUARE_HOLDS_AT_MOST 16u

/* Where one square of a picture sits in the order the console keeps it in.
 *
 * The console does not store a picture in rows. It stores it so that squares
 * near one another on the screen are near one another in memory, because a
 * reading wants a small square of a picture rather than a run of one row. The
 * working out is the hardware's own and every part of it matters: the number is
 * built back up out of pieces that were taken apart to spread neighbouring
 * squares across the parts of memory that can be read at the same time.
 *
 * Counted in whatever the smallest piece of the picture is that can be taken
 * apart on its own: a square of four by four for a picture kept squeezed, and a
 * single part for one that is not. How many bytes one of those comes to is what
 * the count is turned into a place by, so it is said rather than assumed.
 */
static uint32_t where_a_square_sits(uint32_t along, uint32_t down, uint32_t across_in_squares,
                                    unsigned int bytes_a_square) {
    uint32_t far_apart = ((down >> 5) * (across_in_squares >> 5) + (along >> 5)) << 6;
    uint32_t near_together = (((down >> 1) & 7u) << 3) | (along & 7u);
    uint32_t bytes = (far_apart | near_together) << bytes_a_square;

    uint32_t side = (down >> 4) & 1u;
    uint32_t way = ((along >> 3) & 3u) ^ (((down >> 3) & 1u) << 1);
    uint32_t lowest = down & 1u;

    return ((lowest << 4) | (way << 6) | (side << 11)) | (bytes & 0xfu)
           | (((bytes >> 4) & 1u) << 5) | (((bytes >> 5) & 7u) << 8) | ((bytes >> 8) << 12);
}

/* Where in its own square a place sits, worked out once for a square's worth.
 *
 * Where a square sits comes to which square it is in plus where in that square
 * it sits, and the second repeats every two and thirty places across and down.
 * Working the bits out for each place in turn costs more than the copy it is
 * part of, and a frame is copied out many times over, so the repeating part is
 * worked out once and looked up after.
 */
#define XENOLITH_A_SQUARE_ACROSS 32u
#define XENOLITH_WHAT_A_SQUARE_HOLDS (XENOLITH_A_SQUARE_ACROSS * XENOLITH_A_SQUARE_ACROSS * 4u)

static uint32_t where_in_a_square[XENOLITH_A_SQUARE_ACROSS * XENOLITH_A_SQUARE_ACROSS];
static int the_square_is_worked_out;

static void work_out_a_square(void) {
    if (the_square_is_worked_out) {
        return;
    }
    for (uint32_t down = 0; down < XENOLITH_A_SQUARE_ACROSS; down++) {
        for (uint32_t along = 0; along < XENOLITH_A_SQUARE_ACROSS; along++) {
            where_in_a_square[down * XENOLITH_A_SQUARE_ACROSS + along] =
                where_a_square_sits(along, down, XENOLITH_A_SQUARE_ACROSS, 2u);
        }
    }
    the_square_is_worked_out = 1;
}

/* Turns the bytes of a picture the way round a host reads them.
 *
 * The console writes the larger end of a number first and a host reads the
 * smaller end first, and a title says which of the ways its picture was written
 * in. Only the one this title was seen to ask for is turned; anything else is
 * left as it was and said about, because turning bytes the wrong way makes a
 * picture that is wrong quietly. */
static void turn_the_bytes(uint8_t *what, uint32_t how_many, uint32_t order) {
    /* Two apiece and four apiece are the two a title asks for. Which it is says
     * how wide the thing being turned round is, not how much there is of it. */
    if (order == 1u) {
        for (uint32_t at = 0; at + 2u <= how_many; at += 2u) {
            uint8_t held = what[at];
            what[at] = what[at + 1u];
            what[at + 1u] = held;
        }
        return;
    }
    if (order == 2u) {
        for (uint32_t at = 0; at + 4u <= how_many; at += 4u) {
            uint8_t first = what[at];
            uint8_t second = what[at + 1u];
            what[at] = what[at + 3u];
            what[at + 1u] = what[at + 2u];
            what[at + 2u] = second;
            what[at + 3u] = first;
        }
    }
}

/* One thread inside the drawing at a time.
 *
 * Two reach it: the one reading a title's commands, which records the drawing,
 * and whichever guest thread says a frame is done, which ends and hands over
 * what was recorded. They share one pool of commands and one buffer to record
 * into, and a buffer written by two threads at once is not a buffer any device
 * will take. What it looks like when they do is a device that stops: the
 * commands submitted are half of one thread's and half of the other's, the
 * driver cancels the work, and nothing is ever drawn again.
 *
 * Every place that records or hands over takes this, and the one that begins a
 * frame takes it too, so a frame cannot be ended in the middle of being drawn.
 */
/* Recursive: showing a frame is done under this same lock so that a frame
 * being shown cannot record into the command pool a frame being drawn is
 * recording into, and the end-of-frame path that shows already holds it. */
static pthread_mutex_t one_drawing = PTHREAD_RECURSIVE_MUTEX_INITIALIZER_NP;

static int xenolith_begin_a_frame_inside(void);
static int xenolith_end_a_frame_inside(void);
static int hand_over_what_was_recorded(void);
static int send_what_was_recorded(void);
static VkCommandBuffer commands_to_set_up_with(void);
static void hold_until_it_has_run(VkBuffer buffer, VkDeviceMemory memory);
static int a_copy_is_still_to_write(uint32_t lot, uint32_t from, uint32_t over);
static void wait_for_copies_into(uint32_t from, uint32_t over);

/* Lets go of a picture once what has been recorded has run, handing that over
 * first where there is no room left to wait. */
static void let_go_once_it_has_run(VkImage image, VkDeviceMemory memory, VkImageView view) {
    pthread_mutex_lock(&one_drawing);
    /* Kept by the last lot that may read it: the one being recorded, or the
     * one handed over last, which runs after every lot before it. */
    uint32_t lot = batch_open || lots_sent == 0 ? current_lot : last_sent_lot;
    if (letting_go[lot] >= XENOLITH_LET_GO_OF) {
        hand_over_what_was_recorded();
        lot = current_lot;
    }
    to_let_go_of[lot][letting_go[lot]].image = image;
    to_let_go_of[lot][letting_go[lot]].memory = memory;
    to_let_go_of[lot][letting_go[lot]].view = view;
    letting_go[lot]++;
    pthread_mutex_unlock(&one_drawing);
}

/* Keeps the room a picture was handed over through until the lot that hands it
 * over has run. */
static void hold_until_it_has_run(VkBuffer buffer, VkDeviceMemory memory) {
    uint32_t lot = current_lot;
    if (lots[lot].holding >= XENOLITH_HELD_FOR_A_LOT) {
        hand_over_what_was_recorded();
        vkDestroyBuffer(drawing.device, buffer, NULL);
        vkFreeMemory(drawing.device, memory, NULL);
        return;
    }
    lots[lot].held[lots[lot].holding] = buffer;
    lots[lot].held_memory[lots[lot].holding] = memory;
    lots[lot].holding++;
}
static int xenolith_draw_now_inside(uint32_t blending, uint32_t written, uint32_t how_far,
                                    uint32_t cut_off, uint32_t cut_off_at, uint32_t vertex_sum,
                                    const uint32_t *vertex, uint32_t vertex_length,
                                    uint32_t pixel_sum, const uint32_t *pixel,
                                    uint32_t pixel_length, uint32_t vertices, uint32_t kind);
static void xenolith_clip_to_inside(uint32_t left, uint32_t top, uint32_t right, uint32_t bottom);


/* Runs one lot of commands and waits for them.
 *
 * Setting a picture up is the only thing here that has work to do outside a
 * frame, and it happens once for each picture rather than once for each draw,
 * so it waits rather than being woven into what a frame is already doing. */
static VkCommandBuffer begin_one_lot_of_commands(void) {
    if (surface.apart == VK_NULL_HANDLE) {
        return VK_NULL_HANDLE;
    }
    pthread_mutex_lock(&one_lot);

    VkCommandBufferAllocateInfo taking = {0};
    taking.sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO;
    taking.commandPool = surface.apart;
    taking.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY;
    taking.commandBufferCount = 1;

    VkCommandBuffer one = VK_NULL_HANDLE;
    if (vkAllocateCommandBuffers(drawing.device, &taking, &one) != VK_SUCCESS) {
        pthread_mutex_unlock(&one_lot);
        return VK_NULL_HANDLE;
    }

    VkCommandBufferBeginInfo starting = {0};
    starting.sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO;
    starting.flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT;
    if (vkBeginCommandBuffer(one, &starting) != VK_SUCCESS) {
        vkFreeCommandBuffers(drawing.device, surface.apart, 1, &one);
        pthread_mutex_unlock(&one_lot);
        return VK_NULL_HANDLE;
    }
    return one;
}

static int finish_one_lot_of_commands(VkCommandBuffer one) {
    int went = vkEndCommandBuffer(one) == VK_SUCCESS;
    if (went) {
        VkSubmitInfo handing = {0};
        handing.sType = VK_STRUCTURE_TYPE_SUBMIT_INFO;
        handing.commandBufferCount = 1;
        handing.pCommandBuffers = &one;
        pthread_mutex_lock(&one_queue);
        went = vkQueueSubmit(drawing.queue, 1, &handing, VK_NULL_HANDLE) == VK_SUCCESS;
        if (went) {
            vkQueueWaitIdle(drawing.queue);
        }
        pthread_mutex_unlock(&one_queue);
    }
    vkFreeCommandBuffers(drawing.device, surface.apart, 1, &one);
    pthread_mutex_unlock(&one_lot);
    return went;
}

/* Says a picture is now to be used for something other than what it was.
 *
 * A device is told what a picture is about to be for so that it can have it
 * that way before anything reads or writes it. */
static void a_picture_becomes(VkCommandBuffer on, VkImage which, VkImageLayout was,
                              VkImageLayout now) {
    VkImageMemoryBarrier turning = {0};
    turning.sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER;
    turning.oldLayout = was;
    turning.newLayout = now;
    turning.srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
    turning.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
    turning.image = which;
    turning.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
    turning.subresourceRange.levelCount = 1;
    turning.subresourceRange.layerCount = 1;
    turning.srcAccessMask = was == VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL
                                ? VK_ACCESS_TRANSFER_WRITE_BIT
                                : 0;
    turning.dstAccessMask = now == VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL
                                ? VK_ACCESS_TRANSFER_WRITE_BIT
                            : now == VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL
                                ? VK_ACCESS_TRANSFER_READ_BIT
                                : VK_ACCESS_SHADER_READ_BIT;

    vkCmdPipelineBarrier(on, VK_PIPELINE_STAGE_ALL_COMMANDS_BIT, VK_PIPELINE_STAGE_ALL_COMMANDS_BIT,
                         0, 0, NULL, 0, NULL, 1, &turning);
}

/* Makes the one picture a place holding none is read as, and the way every
 * picture is read.
 *
 * A device wants something to look at named in every place whether a shader
 * looks there or not, so one picture of nothing stands in for the places a
 * title has not filled. It is left as it was made, which reads as nothing,
 * because a shader reading a picture a title never set is asking about
 * something that was never there. */
static int make_a_picture_of_nothing(void) {
    if (nothing_seen != VK_NULL_HANDLE) {
        return 1;
    }

    VkSamplerCreateInfo how = {0};
    how.sType = VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO;
    how.magFilter = VK_FILTER_LINEAR;
    how.minFilter = VK_FILTER_LINEAR;
    how.mipmapMode = VK_SAMPLER_MIPMAP_MODE_LINEAR;
    how.addressModeU = VK_SAMPLER_ADDRESS_MODE_REPEAT;
    how.addressModeV = VK_SAMPLER_ADDRESS_MODE_REPEAT;
    how.addressModeW = VK_SAMPLER_ADDRESS_MODE_REPEAT;
    how.maxLod = VK_LOD_CLAMP_NONE;
    if (vkCreateSampler(drawing.device, &how, NULL, &how_a_picture_is_read) != VK_SUCCESS) {
        return 0;
    }

    VkImageCreateInfo wanted = {0};
    wanted.sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO;
    wanted.imageType = VK_IMAGE_TYPE_2D;
    wanted.format = VK_FORMAT_R8G8B8A8_UNORM;
    wanted.extent.width = 1;
    wanted.extent.height = 1;
    wanted.extent.depth = 1;
    wanted.mipLevels = 1;
    wanted.arrayLayers = 1;
    wanted.samples = VK_SAMPLE_COUNT_1_BIT;
    wanted.tiling = VK_IMAGE_TILING_OPTIMAL;
    wanted.usage = VK_IMAGE_USAGE_SAMPLED_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT;
    wanted.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
    if (vkCreateImage(drawing.device, &wanted, NULL, &nothing_to_look_at) != VK_SUCCESS) {
        return 0;
    }

    VkMemoryRequirements needs;
    vkGetImageMemoryRequirements(drawing.device, nothing_to_look_at, &needs);
    VkMemoryAllocateInfo asking = {0};
    asking.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
    asking.allocationSize = needs.size;
    asking.memoryTypeIndex =
        memory_that_suits(needs.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);
    if (vkAllocateMemory(drawing.device, &asking, NULL, &nothing_memory) != VK_SUCCESS
        || vkBindImageMemory(drawing.device, nothing_to_look_at, nothing_memory, 0) != VK_SUCCESS) {
        return 0;
    }

    VkImageViewCreateInfo seeing = {0};
    seeing.sType = VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO;
    seeing.image = nothing_to_look_at;
    seeing.viewType = VK_IMAGE_VIEW_TYPE_2D;
    seeing.format = VK_FORMAT_R8G8B8A8_UNORM;
    seeing.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
    seeing.subresourceRange.levelCount = 1;
    seeing.subresourceRange.layerCount = 1;
    if (vkCreateImageView(drawing.device, &seeing, NULL, &nothing_seen) != VK_SUCCESS) {
        return 0;
    }

    VkCommandBuffer one = begin_one_lot_of_commands();
    if (one == VK_NULL_HANDLE) {
        return 0;
    }

    /* Given a colour rather than left as it was allocated. A shader handed a
     * place a title never filled reads this, and memory nobody wrote holds
     * whatever was last in it, so leaving it alone hands a shader an arbitrary
     * colour that looks exactly like something a title drew. */
    a_picture_becomes(one, nothing_to_look_at, VK_IMAGE_LAYOUT_UNDEFINED,
                      VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL);

    VkClearColorValue nothing = {0};
    VkImageSubresourceRange all_of_it = {0};
    all_of_it.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
    all_of_it.levelCount = 1;
    all_of_it.layerCount = 1;
    vkCmdClearColorImage(one, nothing_to_look_at, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, &nothing, 1,
                         &all_of_it);

    a_picture_becomes(one, nothing_to_look_at, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                      VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL);
    return finish_one_lot_of_commands(one);
}

/* Says which picture stands in each of the places a shader can look at.
 *
 * Every place is named every time, because a device wants all of them named and
 * there is no saying which a shader about to run will look at. */
/* Says once that there is nowhere left to keep a picture already turned. */
static void no_room_for_another_picture(void) {
    static int said;

    if (said) {
        return;
    }
    said = 1;
    fprintf(stderr,
            "xenolith: a title asked for more than %u pictures at once, so one it asks for now "
            "is not drawn with\n",
            XENOLITH_PICTURES_KEPT);
}

static void say_which_pictures_are_looked_at(void) {
    VkDescriptorImageInfo each[XENOLITH_PICTURES];
    for (uint32_t at = 0; at < XENOLITH_PICTURES; at++) {
        each[at].sampler = pictures[at] != 0 ? turned[pictures[at] - 1].way : how_a_picture_is_read;
        each[at].imageView =
            pictures[at] != 0 ? turned[pictures[at] - 1].view : nothing_seen;
        each[at].imageLayout = VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL;
    }

    VkWriteDescriptorSet saying = {0};
    saying.sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET;
    saying.dstSet = through.handed[through.saying_now];
    saying.dstBinding = 2;
    saying.descriptorCount = XENOLITH_PICTURES;
    saying.descriptorType = VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER;
    saying.pImageInfo = each;
    vkUpdateDescriptorSets(drawing.device, 1, &saying, 0, NULL);
}

/* How a title says to mix what it draws with what is already there.
 *
 * The console keeps the two sides and the way of joining them in one register,
 * five bits each for what multiplies the new colour and the old, and three for
 * how the two are put together. Leaving this out draws everything solid, so
 * anything a title meant to show through, the softened edge of a letter most of
 * all, arrives with a hard edge of its own colour.
 */
#define XENOLITH_NOT_MIXING 0x00010001u

static VkBlendFactor mixing_by(uint32_t said) {
    switch (said) {
    case 0:
        return VK_BLEND_FACTOR_ZERO;
    case 1:
        return VK_BLEND_FACTOR_ONE;
    case 4:
        return VK_BLEND_FACTOR_SRC_COLOR;
    case 5:
        return VK_BLEND_FACTOR_ONE_MINUS_SRC_COLOR;
    case 6:
        return VK_BLEND_FACTOR_SRC_ALPHA;
    case 7:
        return VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA;
    case 8:
        return VK_BLEND_FACTOR_DST_COLOR;
    case 9:
        return VK_BLEND_FACTOR_ONE_MINUS_DST_COLOR;
    case 10:
        return VK_BLEND_FACTOR_DST_ALPHA;
    case 11:
        return VK_BLEND_FACTOR_ONE_MINUS_DST_ALPHA;
    case 12:
        return VK_BLEND_FACTOR_CONSTANT_COLOR;
    case 13:
        return VK_BLEND_FACTOR_ONE_MINUS_CONSTANT_COLOR;
    case 14:
        return VK_BLEND_FACTOR_CONSTANT_ALPHA;
    case 15:
        return VK_BLEND_FACTOR_ONE_MINUS_CONSTANT_ALPHA;
    case 16:
        return VK_BLEND_FACTOR_SRC_ALPHA_SATURATE;
    default:
        return VK_BLEND_FACTOR_ONE;
    }
}

static VkBlendOp joining_by(uint32_t said) {
    switch (said) {
    case 1:
        return VK_BLEND_OP_SUBTRACT;
    case 2:
        return VK_BLEND_OP_MIN;
    case 3:
        return VK_BLEND_OP_MAX;
    case 4:
        return VK_BLEND_OP_REVERSE_SUBTRACT;
    default:
        return VK_BLEND_OP_ADD;
    }
}

/* Whether mixing has any effect. Taking the new colour whole and none of the
 * old is what not mixing at all comes to, and saying so lets a device leave the
 * whole step out. */
/* Which parts of a colour a title says to write, four bits for the surface it
 * draws on: red, green, blue and then what is kept of how solid it is.
 *
 * A title turns these off for a pass whose work is not to be seen, and a
 * runtime that writes anyway puts that pass on the screen. This title asks for
 * all four, for none at all, and for all four of each of four surfaces.
 */
#define XENOLITH_WRITE_EVERY_PART 0xfu

static VkColorComponentFlags what_is_written(uint32_t written) {
    VkColorComponentFlags parts = 0;

    if ((written & 1u) != 0) {
        parts |= VK_COLOR_COMPONENT_R_BIT;
    }
    if ((written & 2u) != 0) {
        parts |= VK_COLOR_COMPONENT_G_BIT;
    }
    if ((written & 4u) != 0) {
        parts |= VK_COLOR_COMPONENT_B_BIT;
    }
    if ((written & 8u) != 0) {
        parts |= VK_COLOR_COMPONENT_A_BIT;
    }
    return parts;
}

static int worth_mixing(uint32_t blending) {
    uint32_t colour_from = blending & 0x1fu;
    uint32_t colour_join = (blending >> 5) & 7u;
    uint32_t colour_onto = (blending >> 8) & 0x1fu;
    uint32_t alpha_from = (blending >> 16) & 0x1fu;
    uint32_t alpha_join = (blending >> 21) & 7u;
    uint32_t alpha_onto = (blending >> 24) & 0x1fu;

    return !(colour_from == 1u && colour_onto == 0u && colour_join == 0u && alpha_from == 1u
             && alpha_onto == 0u && alpha_join == 0u);
}

/* What one pair of shaders was made into, with everything else it was built
 * for, kept so the same is not built twice.
 *
 * A title's scene uses hundreds of these and uses them every frame. Room for
 * only the few a menu uses meant that, once it was full, every draw built its
 * own again, which costs more than the drawing does. So it is kept by what it
 * adds up to rather than looked through from the start. */
#define XENOLITH_PIPELINES 4096u
#define XENOLITH_PIPELINE_SAID 10u
static struct {
    uint32_t said[XENOLITH_PIPELINE_SAID];
    VkPipeline pipeline;
    int taken;
} built[XENOLITH_PIPELINES];

/* Where among them one built for this is kept, or where it would go. */
static uint32_t where_a_pipeline_is_kept(const uint32_t *said) {
    uint32_t sum = 0x811c9dc5u;
    for (uint32_t one = 0; one < XENOLITH_PIPELINE_SAID; one++) {
        sum = (sum ^ said[one]) * 0x01000193u;
    }
    uint32_t at = sum % XENOLITH_PIPELINES;
    for (uint32_t tried = 0; tried < XENOLITH_PIPELINES; tried++) {
        if (!built[at].taken || memcmp(built[at].said, said, sizeof built[at].said) == 0) {
            return at;
        }
        at = (at + 1u) % XENOLITH_PIPELINES;
    }
    return XENOLITH_PIPELINES;
}

/* Which register a pixel shader is to find where its pixel is in, or none. */
#define XENOLITH_NO_REGISTER 0xffffffffu
static uint32_t pixel_goes_into = XENOLITH_NO_REGISTER;

void xenolith_find_the_pixel_in(uint32_t which) { pixel_goes_into = which; }

/* How far away a place on the surface is, kept as a plain number between one
 * end of the view and the other.
 *
 * The console keeps twenty four bits of it beside eight of something else, but
 * what is kept here is only ever compared against itself, so the width a host
 * gives it for nothing is the one to take. */
#define XENOLITH_HOW_FAR_FORMAT VK_FORMAT_D32_SFLOAT

/* Asking after a pair of shaders rather than drawing with them, where how far
 * away a thing is has no part in whether the pair can be built at all. */
#define XENOLITH_HOW_FAR_IGNORED 0u

/* Makes somewhere to keep how far away each place on the surface is. */
static int make_somewhere_to_keep_how_far(void) {
    if (through.how_far_seen != VK_NULL_HANDLE) {
        return 1;
    }

    VkImageCreateInfo wanted = {0};
    wanted.sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO;
    wanted.imageType = VK_IMAGE_TYPE_2D;
    wanted.format = XENOLITH_HOW_FAR_FORMAT;
    wanted.extent.width = XENOLITH_GRID_ACROSS;
    wanted.extent.height = XENOLITH_GRID_DOWN;
    wanted.extent.depth = 1;
    wanted.mipLevels = 1;
    wanted.arrayLayers = 1;
    wanted.samples = VK_SAMPLE_COUNT_1_BIT;
    wanted.tiling = VK_IMAGE_TILING_OPTIMAL;
    /* Drawn into, and set to the far end of the view before a frame, which is
     * a transfer into it and has to be asked for separately. */
    wanted.usage = VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT
                   | VK_IMAGE_USAGE_TRANSFER_DST_BIT;
    wanted.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;

    if (vkCreateImage(drawing.device, &wanted, NULL, &through.how_far) != VK_SUCCESS) {
        return 0;
    }

    VkMemoryRequirements needs;
    vkGetImageMemoryRequirements(drawing.device, through.how_far, &needs);

    VkMemoryAllocateInfo asking = {0};
    asking.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
    asking.allocationSize = needs.size;
    asking.memoryTypeIndex =
        memory_that_suits(needs.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);

    if (vkAllocateMemory(drawing.device, &asking, NULL, &through.how_far_memory) != VK_SUCCESS
        || vkBindImageMemory(drawing.device, through.how_far, through.how_far_memory, 0)
               != VK_SUCCESS) {
        return 0;
    }

    VkImageViewCreateInfo seeing = {0};
    seeing.sType = VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO;
    seeing.image = through.how_far;
    seeing.viewType = VK_IMAGE_VIEW_TYPE_2D;
    seeing.format = XENOLITH_HOW_FAR_FORMAT;
    seeing.subresourceRange.aspectMask = VK_IMAGE_ASPECT_DEPTH_BIT;
    seeing.subresourceRange.levelCount = 1;
    seeing.subresourceRange.layerCount = 1;

    return vkCreateImageView(drawing.device, &seeing, NULL, &through.how_far_seen) == VK_SUCCESS;
}

/* Makes what every pipeline shares: somewhere for a frame to go, and what the
 * shaders are told about the world outside them, which is nothing. */
/* Somewhere to put one place's colours while another is drawn into. */
static int somewhere_to_put_a_place_aside(unsigned which) {
    if (put_aside[which].image != VK_NULL_HANDLE) {
        return 1;
    }

    VkImageCreateInfo wanted = {0};
    wanted.sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO;
    wanted.imageType = VK_IMAGE_TYPE_2D;
    wanted.format = XENOLITH_SURFACE_FORMAT;
    wanted.extent.width = XENOLITH_GRID_ACROSS;
    wanted.extent.height = XENOLITH_GRID_DOWN;
    wanted.extent.depth = 1;
    wanted.mipLevels = 1;
    wanted.arrayLayers = 1;
    wanted.samples = VK_SAMPLE_COUNT_1_BIT;
    wanted.tiling = VK_IMAGE_TILING_OPTIMAL;
    wanted.usage = VK_IMAGE_USAGE_TRANSFER_SRC_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT;
    wanted.sharingMode = VK_SHARING_MODE_EXCLUSIVE;
    wanted.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
    if (vkCreateImage(drawing.device, &wanted, NULL, &put_aside[which].image) != VK_SUCCESS) {
        return 0;
    }

    VkMemoryRequirements needs;
    vkGetImageMemoryRequirements(drawing.device, put_aside[which].image, &needs);
    VkMemoryAllocateInfo asking = {0};
    asking.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
    asking.allocationSize = needs.size;
    asking.memoryTypeIndex =
        memory_that_suits(needs.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);
    if (asking.memoryTypeIndex == UINT32_MAX
        || vkAllocateMemory(drawing.device, &asking, NULL, &put_aside[which].memory) != VK_SUCCESS
        || vkBindImageMemory(drawing.device, put_aside[which].image, put_aside[which].memory, 0)
               != VK_SUCCESS) {
        vkDestroyImage(drawing.device, put_aside[which].image, NULL);
        put_aside[which].image = VK_NULL_HANDLE;
        return 0;
    }
    return 1;
}

/* Which of them stands for the shape a title has named, making one where none
 * does and taking the longest unused where there is none to spare. */
static unsigned the_place_of_this_shape(void) {
    unsigned oldest = 0;
    for (unsigned at = 0; at < XENOLITH_PLACES_DRAWN_INTO; at++) {
        if (put_aside[at].taken && put_aside[at].shaped == drawing_into_shape) {
            return at;
        }
        if (!put_aside[at].taken) {
            oldest = at;
            break;
        }
        if (put_aside[at].when < put_aside[oldest].when) {
            oldest = at;
        }
    }
    if (!somewhere_to_put_a_place_aside(oldest)) {
        return XENOLITH_PLACES_DRAWN_INTO;
    }
    put_aside[oldest].taken = 1;
    put_aside[oldest].shaped = drawing_into_shape;
    put_aside[oldest].holds_something = 0;
    put_aside[oldest].across = drawing_across;
    put_aside[oldest].down = 0;
    return oldest;
}

/* Puts the place being drawn into aside and takes up the one a title has turned
 * to, saying whether that one had anything in it already.
 *
 * Both are whole pictures moved on the device, which a title turning between
 * places forty times in a frame pays forty times over. Doing less than this
 * needs a surface, a view and a pass for each place rather than one of each,
 * which is the same moving done by the device's own bookkeeping.
 */
static int surface_brought_back;

static int keep_the_place_and_take_up_another(void) {
    surface_brought_back = 0;
    unsigned wanted = the_place_of_this_shape();
    if (wanted == XENOLITH_PLACES_DRAWN_INTO) {
        return 0;
    }
    put_aside[wanted].when = ++places_turned_to;
    if (wanted == drawing_into_which) {
        /* Still the same place, so the surface already holds what is wanted. */
        return 1;
    }

    VkImageCopy all = {0};
    all.srcSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
    all.srcSubresource.layerCount = 1;
    all.dstSubresource = all.srcSubresource;
    all.extent.depth = 1;

    if (drawing_into_which < XENOLITH_PLACES_DRAWN_INTO) {
        all.extent.width = put_aside[drawing_into_which].across;
        all.extent.height = surface_down != 0 ? surface_down : 1u;
        a_picture_becomes(surface.commands, put_aside[drawing_into_which].image,
                          put_aside[drawing_into_which].holds_something
                              ? VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL
                              : VK_IMAGE_LAYOUT_UNDEFINED,
                          VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL);
        vkCmdCopyImage(surface.commands, surface.image, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                       put_aside[drawing_into_which].image, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                       1, &all);
        a_picture_becomes(surface.commands, put_aside[drawing_into_which].image,
                          VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                          VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL);
        put_aside[drawing_into_which].holds_something = 1;
        put_aside[drawing_into_which].laid = surface_laid;
        put_aside[drawing_into_which].down = all.extent.height;
        anything_recorded = 1;
    }

    int had_something = put_aside[wanted].holds_something;
    surface_laid = had_something ? put_aside[wanted].laid : drawing_laid;
    if (had_something) {
        VkImageMemoryBarrier onto = {0};
        onto.sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER;
        onto.oldLayout = VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
        onto.newLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
        onto.srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
        onto.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
        onto.image = surface.image;
        onto.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
        onto.subresourceRange.levelCount = 1;
        onto.subresourceRange.layerCount = 1;
        onto.srcAccessMask = VK_ACCESS_TRANSFER_READ_BIT;
        onto.dstAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;
        vkCmdPipelineBarrier(surface.commands, VK_PIPELINE_STAGE_TRANSFER_BIT,
                             VK_PIPELINE_STAGE_TRANSFER_BIT, 0, 0, NULL, 0, NULL, 1, &onto);
        all.extent.width = put_aside[wanted].across;
        all.extent.height = put_aside[wanted].down;
        vkCmdCopyImage(surface.commands, put_aside[wanted].image,
                       VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, surface.image,
                       VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1, &all);
        surface_brought_back = 1;
    }
    surface_down = had_something ? put_aside[wanted].down : 0;
    drawing_into_which = wanted;
    return had_something;
}

static int make_what_a_frame_goes_through(void) {
    if (through.made || through.refused) {
        return through.made;
    }
    if (!make_somewhere_to_draw()) {
        through.refused = 1;
        return 0;
    }

    if (!make_somewhere_to_keep_how_far()) {
        through.refused = 1;
        return 0;
    }

    VkAttachmentDescription both[2] = {{0}, {0}};
    VkAttachmentDescription *colour = &both[0];
    colour->format = XENOLITH_SURFACE_FORMAT;
    colour->samples = VK_SAMPLE_COUNT_1_BIT;
    colour->loadOp = VK_ATTACHMENT_LOAD_OP_LOAD;
    colour->storeOp = VK_ATTACHMENT_STORE_OP_STORE;
    colour->stencilLoadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE;
    colour->stencilStoreOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
    colour->initialLayout = VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL;
    colour->finalLayout = VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL;

    VkAttachmentDescription *how_far = &both[1];
    how_far->format = XENOLITH_HOW_FAR_FORMAT;
    how_far->samples = VK_SAMPLE_COUNT_1_BIT;
    how_far->loadOp = VK_ATTACHMENT_LOAD_OP_LOAD;
    how_far->storeOp = VK_ATTACHMENT_STORE_OP_STORE;
    how_far->stencilLoadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE;
    how_far->stencilStoreOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
    how_far->initialLayout = VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL;
    how_far->finalLayout = VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL;

    VkAttachmentReference where = {0};
    where.attachment = 0;
    where.layout = VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL;

    VkAttachmentReference how_far_where = {0};
    how_far_where.attachment = 1;
    how_far_where.layout = VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL;

    VkSubpassDescription only = {0};
    only.pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
    only.colorAttachmentCount = 1;
    only.pColorAttachments = &where;
    only.pDepthStencilAttachment = &how_far_where;

    /* What has to finish before the pass reads what is there.
     *
     * The colour attachment is loaded rather than cleared, because a place a
     * title comes back to holds what it left. What filled it is a transfer that
     * ran before the pass, and without saying so nothing orders the two: a
     * driver is entitled to start the pass first and load whatever the image
     * happened to hold, which is nothing. */
    VkSubpassDependency before[1] = {{0}};
    before[0].srcSubpass = VK_SUBPASS_EXTERNAL;
    before[0].dstSubpass = 0;
    before[0].srcStageMask = VK_PIPELINE_STAGE_TRANSFER_BIT |
                             VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
    before[0].dstStageMask = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT |
                             VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT;
    before[0].srcAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT |
                              VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT;
    before[0].dstAccessMask = VK_ACCESS_COLOR_ATTACHMENT_READ_BIT |
                              VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT |
                              VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT;

    VkRenderPassCreateInfo asking = {0};
    asking.sType = VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO;
    asking.attachmentCount = 2;
    asking.pAttachments = both;
    asking.subpassCount = 1;
    asking.pSubpasses = &only;
    asking.dependencyCount = 1;
    asking.pDependencies = before;

    if (vkCreateRenderPass(drawing.device, &asking, NULL, &through.pass) != VK_SUCCESS) {
        through.refused = 1;
        return 0;
    }

    VkImageViewCreateInfo seeing = {0};
    seeing.sType = VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO;
    seeing.image = surface.image;
    seeing.viewType = VK_IMAGE_VIEW_TYPE_2D;
    seeing.format = XENOLITH_SURFACE_FORMAT;
    seeing.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
    seeing.subresourceRange.levelCount = 1;
    seeing.subresourceRange.layerCount = 1;

    if (vkCreateImageView(drawing.device, &seeing, NULL, &through.view) != VK_SUCCESS) {
        through.refused = 1;
        return 0;
    }

    VkImageView seen[2] = {through.view, through.how_far_seen};

    VkFramebufferCreateInfo holding = {0};
    holding.sType = VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO;
    holding.renderPass = through.pass;
    holding.attachmentCount = 2;
    holding.pAttachments = seen;
    holding.width = XENOLITH_GRID_ACROSS;
    holding.height = XENOLITH_GRID_DOWN;
    holding.layers = 1;

    if (vkCreateFramebuffer(drawing.device, &holding, NULL, &through.target) != VK_SUCCESS) {
        through.refused = 1;
        return 0;
    }

    /* What a shader is handed from outside: a block of constants, a window on
     * the memory a title put its vertices in for a shader to go and get, and
     * the pictures it reads. */
    VkDescriptorSetLayoutBinding all_three[3] = {{0}, {0}, {0}};
    all_three[0].binding = 0;
    all_three[0].descriptorType = VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER;
    all_three[0].descriptorCount = 1;
    all_three[0].stageFlags = VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT;
    all_three[1].binding = 1;
    all_three[1].descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER;
    all_three[1].descriptorCount = 1;
    all_three[1].stageFlags = VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT;
    all_three[2].binding = 2;
    all_three[2].descriptorType = VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER;
    all_three[2].descriptorCount = XENOLITH_PICTURES;
    all_three[2].stageFlags = VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT;

    VkDescriptorSetLayoutCreateInfo shaped = {0};
    shaped.sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO;
    shaped.bindingCount = 3;
    shaped.pBindings = all_three;

    if (vkCreateDescriptorSetLayout(drawing.device, &shaped, NULL, &through.shape) != VK_SUCCESS) {
        through.refused = 1;
        return 0;
    }

    VkPipelineLayoutCreateInfo outside = {0};
    outside.sType = VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO;
    outside.setLayoutCount = 1;
    outside.pSetLayouts = &through.shape;

    if (vkCreatePipelineLayout(drawing.device, &outside, NULL, &through.layout) != VK_SUCCESS) {
        through.refused = 1;
        return 0;
    }

    VkBufferCreateInfo holding_constants = {0};
    holding_constants.sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO;
    holding_constants.size = (VkDeviceSize)XENOLITH_CONSTANT_ROOM * XENOLITH_LOTS;
    holding_constants.usage = VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT;
    holding_constants.sharingMode = VK_SHARING_MODE_EXCLUSIVE;

    if (vkCreateBuffer(drawing.device, &holding_constants, NULL, &through.constants)
        != VK_SUCCESS) {
        through.refused = 1;
        return 0;
    }

    VkBufferCreateInfo holding_vertices = {0};
    holding_vertices.sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO;
    holding_vertices.size = (VkDeviceSize)XENOLITH_VERTEX_ROOM * XENOLITH_LOTS;
    holding_vertices.usage = VK_BUFFER_USAGE_STORAGE_BUFFER_BIT | VK_BUFFER_USAGE_INDEX_BUFFER_BIT;
    holding_vertices.sharingMode = VK_SHARING_MODE_EXCLUSIVE;

    if (vkCreateBuffer(drawing.device, &holding_vertices, NULL, &through.vertices) != VK_SUCCESS) {
        through.refused = 1;
        return 0;
    }

    VkMemoryRequirements wanting;
    vkGetBufferMemoryRequirements(drawing.device, through.vertices, &wanting);

    VkMemoryAllocateInfo asking_too = {0};
    asking_too.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
    asking_too.allocationSize = wanting.size;
    /* Memory a host reads as well as writes, because it does: a run of corners
     * a title names by number is written out in order by reading back the
     * corners already put here. Memory kept out of the cache makes every one of
     * those reads go the whole way to the memory, which was a quarter of the
     * time a frame took. */
    asking_too.memoryTypeIndex = memory_to_read_back_from(wanting.memoryTypeBits);

    if (vkAllocateMemory(drawing.device, &asking_too, NULL, &through.vertex_memory) != VK_SUCCESS
        || vkBindBufferMemory(drawing.device, through.vertices, through.vertex_memory, 0)
               != VK_SUCCESS) {
        through.refused = 1;
        return 0;
    }

    VkMemoryRequirements wants;
    vkGetBufferMemoryRequirements(drawing.device, through.constants, &wants);

    VkMemoryAllocateInfo asking_for = {0};
    asking_for.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
    asking_for.allocationSize = wants.size;
    asking_for.memoryTypeIndex = memory_that_suits(
        wants.memoryTypeBits,
        VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT);

    if (vkAllocateMemory(drawing.device, &asking_for, NULL, &through.constant_memory) != VK_SUCCESS
        || vkBindBufferMemory(drawing.device, through.constants, through.constant_memory, 0)
               != VK_SUCCESS) {
        through.refused = 1;
        return 0;
    }

    /* Mapped once and left so. Every draw writes into them, and mapping and
     * unmapping around each write costs more than the writing does. */
    void *vertex_room = NULL;
    void *constant_room = NULL;
    if (vkMapMemory(drawing.device, through.vertex_memory, 0,
                    (VkDeviceSize)XENOLITH_VERTEX_ROOM * XENOLITH_LOTS, 0, &vertex_room)
            != VK_SUCCESS
        || vkMapMemory(drawing.device, through.constant_memory, 0,
                       (VkDeviceSize)XENOLITH_CONSTANT_ROOM * XENOLITH_LOTS, 0,
                       &constant_room)
               != VK_SUCCESS) {
        through.refused = 1;
        return 0;
    }
    through.vertex_words = vertex_room;
    through.constant_bytes = constant_room;

    VkPhysicalDeviceProperties what_it_is;
    vkGetPhysicalDeviceProperties(drawing.adapter, &what_it_is);
    through.offsets_line_up_on = what_it_is.limits.minUniformBufferOffsetAlignment;
    if (what_it_is.limits.minStorageBufferOffsetAlignment > through.offsets_line_up_on) {
        through.offsets_line_up_on = what_it_is.limits.minStorageBufferOffsetAlignment;
    }
    if (through.offsets_line_up_on == 0) {
        through.offsets_line_up_on = 4u;
    }

    VkDescriptorPoolSize enough[3] = {{0}, {0}, {0}};
    enough[0].type = VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER;
    enough[0].descriptorCount = XENOLITH_SAYINGS * XENOLITH_LOTS;
    enough[1].type = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER;
    enough[1].descriptorCount = XENOLITH_SAYINGS * XENOLITH_LOTS;
    enough[2].type = VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER;
    enough[2].descriptorCount = XENOLITH_PICTURES * XENOLITH_SAYINGS * XENOLITH_LOTS;

    VkDescriptorPoolCreateInfo pool = {0};
    pool.sType = VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO;
    pool.maxSets = XENOLITH_SAYINGS * XENOLITH_LOTS;
    pool.poolSizeCount = 3;
    pool.pPoolSizes = enough;

    if (vkCreateDescriptorPool(drawing.device, &pool, NULL, &through.holding) != VK_SUCCESS) {
        through.refused = 1;
        return 0;
    }

    static VkDescriptorSetLayout every[XENOLITH_SAYINGS * XENOLITH_LOTS];
    for (uint32_t at = 0; at < XENOLITH_SAYINGS * XENOLITH_LOTS; at++) {
        every[at] = through.shape;
    }

    VkDescriptorSetAllocateInfo taking = {0};
    taking.sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO;
    taking.descriptorPool = through.holding;
    taking.descriptorSetCount = XENOLITH_SAYINGS * XENOLITH_LOTS;
    taking.pSetLayouts = every;

    if (vkAllocateDescriptorSets(drawing.device, &taking, through.handed) != VK_SUCCESS) {
        through.refused = 1;
        return 0;
    }

    if (!make_a_picture_of_nothing()) {
        through.refused = 1;
        return 0;
    }

    through.made = 1;
    return 1;
}

/* How big one squeezed square of a picture is, and how many parts across it
 * covers. Both are properties of the one format this reads. */
#define XENOLITH_SQUARE_BYTES 16u
#define XENOLITH_SQUARE_ACROSS 4u

/* The ways this title was seen to keep a picture: squeezed into squares of four
 * by four, and one plain part to a byte. Anything else is reported rather than
 * read as one of these. There are two squeezings that share those squares and
 * differ only in how the see-through part is kept: one interpolates it, the
 * other keeps it plainly a nibble to a part. Both are the same size and layout,
 * so both are read squeezed and only the host format told apart. */
#define XENOLITH_PICTURE_IN_SQUARES 20u
#define XENOLITH_PICTURE_IN_SQUARES_PLAIN 19u
/* Squeezed into squares of half the size, colours only: what a host reads
 * as BC1. The console's number for it. */
#define XENOLITH_PICTURE_IN_SQUARES_SMALL 18u
#define XENOLITH_SMALL_SQUARE_BYTES 8u
/* One part to a texel, kept as a whole float rather than a fraction of one,
 * which a title uses where what it keeps in a picture is a distance rather
 * than a colour. The console's number for it. */
#define XENOLITH_PICTURE_IN_FLOATS 36u
#define XENOLITH_PICTURE_IN_PARTS 2u
#define XENOLITH_PICTURE_IN_FOURS 6u
/* How far away a thing is, kept the way the console keeps it to draw with:
 * three bytes of distance over one of the stencil a title tests against. A
 * title reads one of these as a picture where what it wants is the distance it
 * drew earlier, which is how it works out fog, haze and what is out of focus.
 * The console's numbers for the two of them, one counting a fraction of the
 * whole and the other a small float. */
#define XENOLITH_PICTURE_HOW_FAR 22u
#define XENOLITH_PICTURE_HOW_FAR_AS_A_FLOAT 23u

/* Turns the console's small float, four bits of exponent over twenty of
 * mantissa, into the one a host reads. Biased so that the largest exponent
 * stands for a distance of about one, which is the whole of what a title draws
 * between. */
static float how_far_from_a_short_float(uint32_t held) {
    uint32_t mantissa = held & 0xfffffu;
    int exponent = (int)(held >> 20);
    if (exponent == 0) {
        return ldexpf((float)mantissa / 1048576.0f, -14);
    }
    return ldexpf(1.0f + (float)mantissa / 1048576.0f, exponent - 15);
}

/* Writes out what a picture came to once it has been laid out in rows, where a
 * run says to put them.
 *
 * A picture read out of a title's memory goes through a pitch, an order the
 * bytes sit in and a layout that is not rows, and a mistake in any of those
 * gives a picture that is wrong in a way no register says. This is what was
 * actually made of it, so it can be looked at rather than reasoned about. The
 * squeezed ones are written the way the format they are squeezed in is
 * ordinarily kept, so anything that reads that format reads these.
 */
static void keep_what_a_picture_came_to(uint32_t where, uint32_t across, uint32_t down,
                                        uint32_t bytes, int squeezed, int in_fours,
                                        const uint8_t *rows) {
    const char *into = getenv("XENOLITH_KEEP_PICTURES");
    if (into == NULL) {
        return;
    }

    char name[512];
    if (snprintf(name, sizeof name, "%s/%08x-%ux%u.%s", into, where, across, down,
                 squeezed ? "dds" : (in_fours ? "ppm" : "pgm"))
        >= (int)sizeof name) {
        return;
    }
    FILE *out = fopen(name, "wb");
    if (out == NULL) {
        return;
    }

    if (squeezed) {
        /* The header the squeezed format is ordinarily kept behind, filled in
         * with the least that names one picture of one size with no smaller
         * copies beside it. */
        uint32_t head[32] = {0};
        head[0] = 0x20534444u;
        head[1] = 124u;
        head[2] = 0x000a1007u;
        head[3] = down;
        head[4] = across;
        head[5] = bytes;
        head[7] = 1u;
        head[19] = 32u;
        head[20] = 4u;
        head[21] = 0x35545844u;
        head[27] = 0x1000u;
        fwrite(head, 4, 32, out);
        fwrite(rows, 1, bytes, out);
    } else {
        fprintf(out, "%s\n%u %u\n255\n", in_fours ? "P6" : "P5", across, down);
        for (uint32_t step = 0; step < across * down; step++) {
            if (in_fours) {
                fwrite(rows + (size_t)step * 4u, 1, 3, out);
            } else {
                fputc(rows[step], out);
            }
        }
    }
    fclose(out);
}

int xenolith_look_at_a_picture(uint32_t slot, const uint8_t *from, uint32_t where, uint32_t across,
                               uint32_t down, uint32_t pitch, int in_squares, uint32_t order,
                               uint32_t kept, uint32_t held_how) {
    if (slot >= XENOLITH_PICTURES || across == 0 || down == 0
        || !make_what_a_frame_goes_through()) {
        return 0;
    }

    /* One already turned is not turned again, whichever place it was in when
     * it was. A title names its pictures before every draw and moves the same
     * few between the places it reads them from. */
    for (uint32_t at = 0; at < XENOLITH_PICTURES_KEPT; at++) {
        if (turned[at].held && turned[at].from == where && turned[at].across == across
            && turned[at].down == down && turned[at].kept == kept
            && turned[at].held_how == held_how) {
            /* Checked once a frame. A title writing over a picture through a
             * copy is caught by forgetting it outright, and one writing over it
             * with the processor is caught at the next frame. Checking every
             * draw instead reads the whole of every picture every time it is
             * read from, which came to a hundred and twenty nine gigabytes in
             * one mission to find three thousand changes. */
            /* One a copy keeps putting into on the device is not added up at
             * all: its memory is what the copy wrote, and adding up a frame's
             * worth of whole scenes after every copy cost more than the
             * copies. Once copies stop going into it, what its memory adds up
             * to is worked out the next time it is checked. */
            int written_over = turned[at].stale;
            int kept_up_by_copies =
                turned[at].renewed_in != 0 && turned[at].renewed_in + 2u >= batches_begun;
            if (!written_over && turned[at].over != 0 && turned[at].checked_in != frames_shown
                && !kept_up_by_copies) {
                uint32_t adds_up_to = what_the_memory_adds_up_to(from, where, turned[at].over);
                if (turned[at].came_to_unknown) {
                    turned[at].came_to = adds_up_to;
                    turned[at].came_to_unknown = 0;
                } else {
                    written_over = adds_up_to != turned[at].came_to;
                }
            }
            if (written_over) {
                /* Written over since. It is turned again, and the one that was
                 * there waits to be let go of until what was recorded with it
                 * has run. */
                let_go_once_it_has_run(turned[at].image, turned[at].memory, turned[at].view);

                for (uint32_t place = 0; place < XENOLITH_PICTURES; place++) {
                    if (pictures[place] == at + 1u) {
                        pictures[place] = 0;
                    }
                }
                memset(&turned[at], 0, sizeof turned[at]);
                break;
            }
            turned[at].checked_in = frames_shown;
            pictures[slot] = at + 1;
            return 1;
        }
    }

    /* What the smallest piece of this picture is, which is a square of four by
     * four where it is kept squeezed and a single part where it is not, and how
     * many bytes one of those comes to. */
    int small = kept == XENOLITH_PICTURE_IN_SQUARES_SMALL;
    int squeezed = small || kept == XENOLITH_PICTURE_IN_SQUARES
                   || kept == XENOLITH_PICTURE_IN_SQUARES_PLAIN;
    int in_fours = kept == XENOLITH_PICTURE_IN_FOURS;
    int in_floats = kept == XENOLITH_PICTURE_IN_FLOATS;
    int how_far = kept == XENOLITH_PICTURE_HOW_FAR || kept == XENOLITH_PICTURE_HOW_FAR_AS_A_FLOAT;
    uint32_t square_across = squeezed ? XENOLITH_SQUARE_ACROSS : 1u;
    /* The small squeezing keeps a square in half the bytes, having no
     * see-through part of its own beyond the one bit its colours can spare. */
    uint32_t square_bytes = squeezed ? (small ? XENOLITH_SMALL_SQUARE_BYTES : XENOLITH_SQUARE_BYTES)
                                     : (in_fours || in_floats || how_far ? 4u : 1u);
    unsigned int bytes_a_square =
        squeezed ? (small ? 3u : 4u) : (in_fours || in_floats || how_far ? 2u : 0u);
    /* A console names the parts of a colour the way round it writes them, which
     * is the way a host reads them once the bytes have been turned. The three
     * squeezings share the squares and differ only here: the small one has
     * only its colours, which is what a host reads as BC1; the plain one keeps
     * its see-through part a nibble to a part, which is BC2; and the other
     * interpolates it, which is BC3. */
    VkFormat as_a_host_reads_it =
        small ? VK_FORMAT_BC1_RGBA_UNORM_BLOCK
        : kept == XENOLITH_PICTURE_IN_SQUARES_PLAIN
            ? VK_FORMAT_BC2_UNORM_BLOCK
            : (squeezed ? VK_FORMAT_BC3_UNORM_BLOCK
                        : (in_floats || how_far ? VK_FORMAT_R32_SFLOAT
                                     : (in_fours ? VK_FORMAT_R8G8B8A8_UNORM
                                                 : VK_FORMAT_R8_UNORM)));

    uint32_t wide = (across + square_across - 1u) / square_across;
    uint32_t tall = (down + square_across - 1u) / square_across;
    uint32_t along = (pitch + square_across - 1u) / square_across;
    if (along < wide) {
        along = wide;
    }
    /* The order the console keeps a picture in is worked out in lots of
     * thirty two, so the run it is stored in is as wide as the next such lot. */
    along = (along + 31u) & ~31u;

    VkDeviceSize enough = (VkDeviceSize)wide * tall * square_bytes;

    /* A copy recorded but not yet run is still to write this memory, and the
     * picture has to be what it writes rather than what was there before. */
    wait_for_copies_into(where, along * ((tall + 31u) & ~31u) * square_bytes);

    VkBufferCreateInfo holding = {0};
    holding.sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO;
    holding.size = enough;
    holding.usage = VK_BUFFER_USAGE_TRANSFER_SRC_BIT;
    holding.sharingMode = VK_SHARING_MODE_EXCLUSIVE;

    VkBuffer through_it = VK_NULL_HANDLE;
    VkDeviceMemory its_memory = VK_NULL_HANDLE;
    if (vkCreateBuffer(drawing.device, &holding, NULL, &through_it) != VK_SUCCESS) {
        return 0;
    }

    VkMemoryRequirements needs;
    vkGetBufferMemoryRequirements(drawing.device, through_it, &needs);
    VkMemoryAllocateInfo asking = {0};
    asking.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
    asking.allocationSize = needs.size;
    asking.memoryTypeIndex = memory_that_suits(
        needs.memoryTypeBits,
        VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT);
    if (vkAllocateMemory(drawing.device, &asking, NULL, &its_memory) != VK_SUCCESS
        || vkBindBufferMemory(drawing.device, through_it, its_memory, 0) != VK_SUCCESS) {
        vkDestroyBuffer(drawing.device, through_it, NULL);
        vkFreeMemory(drawing.device, its_memory, NULL);
        return 0;
    }

    void *into = NULL;
    if (vkMapMemory(drawing.device, its_memory, 0, enough, 0, &into) != VK_SUCCESS
        || into == NULL) {
        vkDestroyBuffer(drawing.device, through_it, NULL);
        vkFreeMemory(drawing.device, its_memory, NULL);
        return 0;
    }

    /* Laid out in rows, one square at a time, out of wherever the console kept
     * that square.
     *
     * Where a square sits comes to which lot of two and thirty it is in plus
     * where in that lot it sits, and the second repeats. Working the bits out
     * for every square of a picture was costing more than everything else a
     * picture needs put together, so the repeating part is worked out once for
     * a lot's worth and the rest is two multiplies and a look up. */
    /* Where each square of a lot sits depends on nothing but how many bytes a
     * square takes, so it is worked out once for each width a title uses rather
     * than once for every picture handed over. A title hands pictures over by
     * the hundred in a frame and uses a handful of widths in a whole run. */
    static uint32_t in_the_lot_by_width[XENOLITH_A_SQUARE_HOLDS_AT_MOST + 1u]
                                       [XENOLITH_A_SQUARE_ACROSS * XENOLITH_A_SQUARE_ACROSS];
    static int worked_out_for[XENOLITH_A_SQUARE_HOLDS_AT_MOST + 1u];

    const uint32_t *in_the_lot = NULL;
    uint32_t a_lot_along = 0;
    uint32_t a_lot_down = 0;
    if (in_squares && bytes_a_square <= XENOLITH_A_SQUARE_HOLDS_AT_MOST) {
        uint32_t *lot = in_the_lot_by_width[bytes_a_square];
        if (!worked_out_for[bytes_a_square]) {
            for (uint32_t down_one = 0; down_one < XENOLITH_A_SQUARE_ACROSS; down_one++) {
                for (uint32_t along_one = 0; along_one < XENOLITH_A_SQUARE_ACROSS; along_one++) {
                    lot[down_one * XENOLITH_A_SQUARE_ACROSS + along_one] =
                        where_a_square_sits(along_one, down_one, XENOLITH_A_SQUARE_ACROSS,
                                            bytes_a_square);
                }
            }
            worked_out_for[bytes_a_square] = 1;
        }
        in_the_lot = lot;
        uint32_t at_nought = where_a_square_sits(0, 0, along, bytes_a_square);
        a_lot_along = where_a_square_sits(XENOLITH_A_SQUARE_ACROSS, 0, along, bytes_a_square)
                      - at_nought;
        a_lot_down = where_a_square_sits(0, XENOLITH_A_SQUARE_ACROSS, along, bytes_a_square)
                     - at_nought;
    } else if (in_squares) {
        return 0;
    }

    uint8_t *rows = into;
    for (uint32_t down_one = 0; down_one < tall; down_one++) {
        uint32_t down_to = (down_one / XENOLITH_A_SQUARE_ACROSS) * a_lot_down;
        const uint32_t *in_the_row =
            &in_the_lot[(down_one % XENOLITH_A_SQUARE_ACROSS) * XENOLITH_A_SQUARE_ACROSS];
        uint8_t *put = rows + (VkDeviceSize)down_one * wide * square_bytes;

        for (uint32_t along_one = 0; along_one < wide; along_one++, put += square_bytes) {
            uint32_t sits =
                in_squares
                    ? down_to + (along_one / XENOLITH_A_SQUARE_ACROSS) * a_lot_along
                          + in_the_row[along_one % XENOLITH_A_SQUARE_ACROSS]
                    : (down_one * along + along_one) * square_bytes;
            const uint8_t *took = from + where + sits;

            /* Four bytes turned four at a time is nearly every square a title
             * hands over, and going through the general way round for each one
             * of them costs more than everything else a picture needs. */
            if (how_far) {
                uint32_t held;
                if (order == 2u) {
                    /* Turning four bytes round and then reading them the way a
                     * host does is reading them the way the console wrote them,
                     * so it is done in one go. Every distance a title hands over
                     * arrives this way, tens of millions of them in a mission,
                     * and going the general way round for each was costing more
                     * than the rest of the picture. */
                    held = (uint32_t)took[0] << 24 | (uint32_t)took[1] << 16
                           | (uint32_t)took[2] << 8 | (uint32_t)took[3];
                } else {
                    uint8_t four[4];
                    memcpy(four, took, sizeof four);
                    turn_the_bytes(four, sizeof four, order);
                    memcpy(&held, four, sizeof held);
                }
                /* The stencil is the low byte and is nothing to a shader
                 * reading a distance, so it goes. */
                held >>= 8;
                float away = kept == XENOLITH_PICTURE_HOW_FAR_AS_A_FLOAT
                                 ? how_far_from_a_short_float(held)
                                 : (float)held / 16777215.0f;
                memcpy(put, &away, sizeof away);
                continue;
            }
            if (square_bytes == 4u && order == 2u) {
                put[0] = took[3];
                put[1] = took[2];
                put[2] = took[1];
                put[3] = took[0];
                continue;
            }
            memcpy(put, took, square_bytes);
            turn_the_bytes(put, square_bytes, order);
        }
    }
    keep_what_a_picture_came_to(where, wide * square_across, tall * square_across,
                                (uint32_t)enough, squeezed, in_fours, rows);
    vkUnmapMemory(drawing.device, its_memory);

    VkImageCreateInfo wanted = {0};
    wanted.sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO;
    wanted.imageType = VK_IMAGE_TYPE_2D;
    wanted.format = as_a_host_reads_it;
    wanted.extent.width = wide * square_across;
    wanted.extent.height = tall * square_across;
    wanted.extent.depth = 1;
    wanted.mipLevels = 1;
    wanted.arrayLayers = 1;
    wanted.samples = VK_SAMPLE_COUNT_1_BIT;
    wanted.tiling = VK_IMAGE_TILING_OPTIMAL;
    wanted.usage = VK_IMAGE_USAGE_TRANSFER_DST_BIT | VK_IMAGE_USAGE_SAMPLED_BIT;
    wanted.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;

    VkImage made = VK_NULL_HANDLE;
    VkDeviceMemory its_own = VK_NULL_HANDLE;
    VkImageView seen = VK_NULL_HANDLE;
    int went = vkCreateImage(drawing.device, &wanted, NULL, &made) == VK_SUCCESS;
    if (went) {
        vkGetImageMemoryRequirements(drawing.device, made, &needs);
        asking.allocationSize = needs.size;
        asking.memoryTypeIndex =
            memory_that_suits(needs.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);
        went = vkAllocateMemory(drawing.device, &asking, NULL, &its_own) == VK_SUCCESS
               && vkBindImageMemory(drawing.device, made, its_own, 0) == VK_SUCCESS;
    }

    /* Handed over with the lot being recorded, just ahead of its frames, and
     * the room it is handed over through kept until that lot has run. */
    int handed_over = 0;
    if (went) {
        pthread_mutex_lock(&one_drawing);
        VkCommandBuffer one = commands_to_set_up_with();
        went = one != VK_NULL_HANDLE;
        if (went) {
            a_picture_becomes(one, made, VK_IMAGE_LAYOUT_UNDEFINED,
                              VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL);

            VkBufferImageCopy putting = {0};
            putting.imageSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
            putting.imageSubresource.layerCount = 1;
            putting.imageExtent = wanted.extent;
            vkCmdCopyBufferToImage(one, through_it, made, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1,
                                   &putting);

            a_picture_becomes(one, made, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                              VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL);
            hold_until_it_has_run(through_it, its_memory);
            handed_over = 1;
        }
        pthread_mutex_unlock(&one_drawing);
    }

    if (went) {
        VkImageViewCreateInfo seeing = {0};
        seeing.sType = VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO;
        seeing.image = made;
        seeing.viewType = VK_IMAGE_VIEW_TYPE_2D;
        seeing.format = as_a_host_reads_it;
        seeing.components.r = which_part_is_read(held_how >> XENOLITH_PARTS_SHIFT);
        seeing.components.g = which_part_is_read(held_how >> (XENOLITH_PARTS_SHIFT + 3u));
        seeing.components.b = which_part_is_read(held_how >> (XENOLITH_PARTS_SHIFT + 6u));
        seeing.components.a = which_part_is_read(held_how >> (XENOLITH_PARTS_SHIFT + 9u));
        seeing.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
        seeing.subresourceRange.levelCount = 1;
        seeing.subresourceRange.layerCount = 1;
        went = vkCreateImageView(drawing.device, &seeing, NULL, &seen) == VK_SUCCESS;
    }

    if (!handed_over) {
        vkDestroyBuffer(drawing.device, through_it, NULL);
        vkFreeMemory(drawing.device, its_memory, NULL);
    }

    if (!went) {
        vkDestroyImageView(drawing.device, seen, NULL);
        vkDestroyImage(drawing.device, made, NULL);
        vkFreeMemory(drawing.device, its_own, NULL);
        return 0;
    }

    /* Somewhere to keep it. A title's pictures are finite and few, so running
     * out means something is being turned that should have been recognised,
     * which is said rather than worked around by throwing one away: what would
     * be thrown away may still be being read from. */
    uint32_t room = XENOLITH_PICTURES_KEPT;
    for (uint32_t at = 0; at < XENOLITH_PICTURES_KEPT; at++) {
        if (!turned[at].held) {
            room = at;
            break;
        }
    }
    /* Where every place is taken, one written over is given up for this. */
    for (uint32_t at = 0; at < XENOLITH_PICTURES_KEPT && room == XENOLITH_PICTURES_KEPT; at++) {
        if (turned[at].stale) {
            let_go_once_it_has_run(turned[at].image, turned[at].memory, turned[at].view);
            memset(&turned[at], 0, sizeof turned[at]);
            room = at;
        }
    }
    if (room == XENOLITH_PICTURES_KEPT) {
        no_room_for_another_picture();
        vkDestroyImageView(drawing.device, seen, NULL);
        vkDestroyImage(drawing.device, made, NULL);
        vkFreeMemory(drawing.device, its_own, NULL);
        return 0;
    }

    turned[room].image = made;
    turned[room].memory = its_own;
    turned[room].view = seen;
    turned[room].from = where;
    turned[room].across = across;
    turned[room].down = down;
    turned[room].kept = kept;
    turned[room].held_how = held_how;
    turned[room].in_squares = in_squares;
    turned[room].along = along;
    turned[room].order = order;
    turned[room].renewed_by = 0;
    turned[room].renewed_in = 0;
    turned[room].came_to_unknown = 0;
    turned[room].stale = 0;
    turned[room].way = a_way_of_reading(held_how & XENOLITH_HOW_IT_IS_READ);
    /* How much of a title's memory this was read out of, which is not how much
     * it came to. Where it is kept in squares the run it sits in is as wide as
     * the pitch and as tall as the next whole lot of two and thirty, so one
     * narrower than its pitch is spread over more memory than it fills. Adding
     * up only what it fills leaves the rest unwatched, and a title writing a
     * letter into that part of one goes unnoticed. */
    uint32_t read_over = in_squares ? along * ((tall + 31u) & ~31u) * square_bytes
                                    : (uint32_t)enough;
    turned[room].over = read_over;
    turned[room].came_to = what_the_memory_adds_up_to(from, where, read_over);
    turned[room].held = 1;
    pictures[slot] = room + 1;

    return 1;
}

/* Turns a run of words into something a device holds. */
/* What finishes a rectangle on the device: handed the three corners a title
 * gave, after its vertex shader has run on each, it passes them on and then
 * the second triangle, whose new corner is across from the right angle, every
 * part of it worked out as the two corners beside it less the one across. The
 * right angle is the corner across from the longest side as it lands on the
 * screen, which is the one a title names wherever it starts. Both triangles
 * are wound the way the three corners were. Made once, outside the build. */
static const uint32_t the_rest_of_a_rectangle[] = {
    0x07230203u, 0x00010000u, 0x0008000bu, 0x000000b7u, 0x00000000u, 0x00020011u,
    0x00000002u, 0x0006000bu, 0x00000001u, 0x4c534c47u, 0x6474732eu, 0x3035342eu,
    0x00000000u, 0x0003000eu, 0x00000000u, 0x00000001u, 0x0009000fu, 0x00000003u,
    0x00000004u, 0x6e69616du, 0x00000000u, 0x00000015u, 0x0000001cu, 0x00000030u,
    0x00000034u, 0x00030010u, 0x00000004u, 0x00000016u, 0x00040010u, 0x00000004u,
    0x00000000u, 0x00000001u, 0x00030010u, 0x00000004u, 0x0000001du, 0x00040010u,
    0x00000004u, 0x0000001au, 0x00000006u, 0x00030047u, 0x00000013u, 0x00000002u,
    0x00050048u, 0x00000013u, 0x00000000u, 0x0000000bu, 0x00000000u, 0x00030047u,
    0x00000017u, 0x00000002u, 0x00050048u, 0x00000017u, 0x00000000u, 0x0000000bu,
    0x00000000u, 0x00040047u, 0x00000030u, 0x0000001eu, 0x00000000u, 0x00040047u,
    0x00000034u, 0x0000001eu, 0x00000000u, 0x00020013u, 0x00000002u, 0x00030021u,
    0x00000003u, 0x00000002u, 0x00040015u, 0x00000006u, 0x00000020u, 0x00000001u,
    0x00040020u, 0x00000007u, 0x00000007u, 0x00000006u, 0x00040021u, 0x00000008u,
    0x00000002u, 0x00000007u, 0x00030016u, 0x0000000cu, 0x00000020u, 0x00040017u,
    0x0000000du, 0x0000000cu, 0x00000002u, 0x00040021u, 0x0000000eu, 0x0000000du,
    0x00000007u, 0x00040017u, 0x00000012u, 0x0000000cu, 0x00000004u, 0x0003001eu,
    0x00000013u, 0x00000012u, 0x00040020u, 0x00000014u, 0x00000003u, 0x00000013u,
    0x0004003bu, 0x00000014u, 0x00000015u, 0x00000003u, 0x0004002bu, 0x00000006u,
    0x00000016u, 0x00000000u, 0x0003001eu, 0x00000017u, 0x00000012u, 0x00040015u,
    0x00000018u, 0x00000020u, 0x00000000u, 0x0004002bu, 0x00000018u, 0x00000019u,
    0x00000003u, 0x0004001cu, 0x0000001au, 0x00000017u, 0x00000019u, 0x00040020u,
    0x0000001bu, 0x00000001u, 0x0000001au, 0x0004003bu, 0x0000001bu, 0x0000001cu,
    0x00000001u, 0x00040020u, 0x0000001eu, 0x00000001u, 0x00000012u, 0x00040020u,
    0x00000021u, 0x00000003u, 0x00000012u, 0x0004002bu, 0x00000006u, 0x0000002au,
    0x00000010u, 0x00020014u, 0x0000002bu, 0x0004002bu, 0x00000018u, 0x0000002du,
    0x00000010u, 0x0004001cu, 0x0000002eu, 0x00000012u, 0x0000002du, 0x00040020u,
    0x0000002fu, 0x00000003u, 0x0000002eu, 0x0004003bu, 0x0000002fu, 0x00000030u,
    0x00000003u, 0x0004001cu, 0x00000032u, 0x0000002eu, 0x00000019u, 0x00040020u,
    0x00000033u, 0x00000001u, 0x00000032u, 0x0004003bu, 0x00000033u, 0x00000034u,
    0x00000001u, 0x0004002bu, 0x00000006u, 0x0000003bu, 0x00000001u, 0x00040020u,
    0x0000003du, 0x00000007u, 0x00000012u, 0x00040020u, 0x00000042u, 0x00000007u,
    0x0000000cu, 0x0004002bu, 0x0000000cu, 0x00000046u, 0x358637bdu, 0x00040020u,
    0x00000048u, 0x00000007u, 0x0000000du, 0x0004002bu, 0x00000006u, 0x0000005bu,
    0x00000002u, 0x0004002bu, 0x00000006u, 0x0000007fu, 0x00000003u, 0x00050036u,
    0x00000002u, 0x00000004u, 0x00000000u, 0x00000003u, 0x000200f8u, 0x00000005u,
    0x0004003bu, 0x00000042u, 0x00000058u, 0x00000007u, 0x0004003bu, 0x00000007u,
    0x00000059u, 0x00000007u, 0x0004003bu, 0x00000007u, 0x0000005cu, 0x00000007u,
    0x0004003bu, 0x00000042u, 0x0000005fu, 0x00000007u, 0x0004003bu, 0x00000007u,
    0x00000060u, 0x00000007u, 0x0004003bu, 0x00000007u, 0x00000062u, 0x00000007u,
    0x0004003bu, 0x00000042u, 0x00000065u, 0x00000007u, 0x0004003bu, 0x00000007u,
    0x00000066u, 0x00000007u, 0x0004003bu, 0x00000007u, 0x00000068u, 0x00000007u,
    0x0004003bu, 0x00000007u, 0x0000006bu, 0x00000007u, 0x0004003bu, 0x00000007u,
    0x00000073u, 0x00000007u, 0x0004003bu, 0x00000007u, 0x0000007cu, 0x00000007u,
    0x0004003bu, 0x00000007u, 0x00000081u, 0x00000007u, 0x0004003bu, 0x00000007u,
    0x00000085u, 0x00000007u, 0x0004003bu, 0x00000007u, 0x00000087u, 0x00000007u,
    0x0004003bu, 0x00000007u, 0x00000089u, 0x00000007u, 0x0004003bu, 0x00000007u,
    0x0000008bu, 0x00000007u, 0x0004003bu, 0x00000007u, 0x0000009au, 0x00000007u,
    0x0004003bu, 0x00000007u, 0x000000b4u, 0x00000007u, 0x0003003eu, 0x00000059u,
    0x0000003bu, 0x00050039u, 0x0000000du, 0x0000005au, 0x00000010u, 0x00000059u,
    0x0003003eu, 0x0000005cu, 0x0000005bu, 0x00050039u, 0x0000000du, 0x0000005du,
    0x00000010u, 0x0000005cu, 0x0007000cu, 0x0000000cu, 0x0000005eu, 0x00000001u,
    0x00000043u, 0x0000005au, 0x0000005du, 0x0003003eu, 0x00000058u, 0x0000005eu,
    0x0003003eu, 0x00000060u, 0x0000005bu, 0x00050039u, 0x0000000du, 0x00000061u,
    0x00000010u, 0x00000060u, 0x0003003eu, 0x00000062u, 0x00000016u, 0x00050039u,
    0x0000000du, 0x00000063u, 0x00000010u, 0x00000062u, 0x0007000cu, 0x0000000cu,
    0x00000064u, 0x00000001u, 0x00000043u, 0x00000061u, 0x00000063u, 0x0003003eu,
    0x0000005fu, 0x00000064u, 0x0003003eu, 0x00000066u, 0x00000016u, 0x00050039u,
    0x0000000du, 0x00000067u, 0x00000010u, 0x00000066u, 0x0003003eu, 0x00000068u,
    0x0000003bu, 0x00050039u, 0x0000000du, 0x00000069u, 0x00000010u, 0x00000068u,
    0x0007000cu, 0x0000000cu, 0x0000006au, 0x00000001u, 0x00000043u, 0x00000067u,
    0x00000069u, 0x0003003eu, 0x00000065u, 0x0000006au, 0x0004003du, 0x0000000cu,
    0x0000006cu, 0x00000058u, 0x0004003du, 0x0000000cu, 0x0000006du, 0x0000005fu,
    0x000500beu, 0x0000002bu, 0x0000006eu, 0x0000006cu, 0x0000006du, 0x0004003du,
    0x0000000cu, 0x0000006fu, 0x00000058u, 0x0004003du, 0x0000000cu, 0x00000070u,
    0x00000065u, 0x000500beu, 0x0000002bu, 0x00000071u, 0x0000006fu, 0x00000070u,
    0x000500a7u, 0x0000002bu, 0x00000072u, 0x0000006eu, 0x00000071u, 0x000300f7u,
    0x00000075u, 0x00000000u, 0x000400fau, 0x00000072u, 0x00000074u, 0x00000076u,
    0x000200f8u, 0x00000074u, 0x0003003eu, 0x00000073u, 0x00000016u, 0x000200f9u,
    0x00000075u, 0x000200f8u, 0x00000076u, 0x0004003du, 0x0000000cu, 0x00000077u,
    0x0000005fu, 0x0004003du, 0x0000000cu, 0x00000078u, 0x00000065u, 0x000500beu,
    0x0000002bu, 0x00000079u, 0x00000077u, 0x00000078u, 0x000600a9u, 0x00000006u,
    0x0000007au, 0x00000079u, 0x0000003bu, 0x0000005bu, 0x0003003eu, 0x00000073u,
    0x0000007au, 0x000200f9u, 0x00000075u, 0x000200f8u, 0x00000075u, 0x0004003du,
    0x00000006u, 0x0000007bu, 0x00000073u, 0x0003003eu, 0x0000006bu, 0x0000007bu,
    0x0004003du, 0x00000006u, 0x0000007du, 0x0000006bu, 0x00050080u, 0x00000006u,
    0x0000007eu, 0x0000007du, 0x0000003bu, 0x0005008bu, 0x00000006u, 0x00000080u,
    0x0000007eu, 0x0000007fu, 0x0003003eu, 0x0000007cu, 0x00000080u, 0x0004003du,
    0x00000006u, 0x00000082u, 0x0000006bu, 0x00050080u, 0x00000006u, 0x00000083u,
    0x00000082u, 0x0000005bu, 0x0005008bu, 0x00000006u, 0x00000084u, 0x00000083u,
    0x0000007fu, 0x0003003eu, 0x00000081u, 0x00000084u, 0x0003003eu, 0x00000085u,
    0x00000016u, 0x00050039u, 0x00000002u, 0x00000086u, 0x0000000au, 0x00000085u,
    0x0003003eu, 0x00000087u, 0x0000003bu, 0x00050039u, 0x00000002u, 0x00000088u,
    0x0000000au, 0x00000087u, 0x0003003eu, 0x00000089u, 0x0000005bu, 0x00050039u,
    0x00000002u, 0x0000008au, 0x0000000au, 0x00000089u, 0x000100dbu, 0x0004003du,
    0x00000006u, 0x0000008cu, 0x0000007cu, 0x0003003eu, 0x0000008bu, 0x0000008cu,
    0x00050039u, 0x00000002u, 0x0000008du, 0x0000000au, 0x0000008bu, 0x0004003du,
    0x00000006u, 0x0000008eu, 0x0000007cu, 0x00060041u, 0x0000001eu, 0x0000008fu,
    0x0000001cu, 0x0000008eu, 0x00000016u, 0x0004003du, 0x00000012u, 0x00000090u,
    0x0000008fu, 0x0004003du, 0x00000006u, 0x00000091u, 0x00000081u, 0x00060041u,
    0x0000001eu, 0x00000092u, 0x0000001cu, 0x00000091u, 0x00000016u, 0x0004003du,
    0x00000012u, 0x00000093u, 0x00000092u, 0x00050081u, 0x00000012u, 0x00000094u,
    0x00000090u, 0x00000093u, 0x0004003du, 0x00000006u, 0x00000095u, 0x0000006bu,
    0x00060041u, 0x0000001eu, 0x00000096u, 0x0000001cu, 0x00000095u, 0x00000016u,
    0x0004003du, 0x00000012u, 0x00000097u, 0x00000096u, 0x00050083u, 0x00000012u,
    0x00000098u, 0x00000094u, 0x00000097u, 0x00050041u, 0x00000021u, 0x00000099u,
    0x00000015u, 0x00000016u, 0x0003003eu, 0x00000099u, 0x00000098u, 0x0003003eu,
    0x0000009au, 0x00000016u, 0x000200f9u, 0x0000009bu, 0x000200f8u, 0x0000009bu,
    0x000400f6u, 0x0000009du, 0x0000009eu, 0x00000000u, 0x000200f9u, 0x0000009fu,
    0x000200f8u, 0x0000009fu, 0x0004003du, 0x00000006u, 0x000000a0u, 0x0000009au,
    0x000500b1u, 0x0000002bu, 0x000000a1u, 0x000000a0u, 0x0000002au, 0x000400fau,
    0x000000a1u, 0x0000009cu, 0x0000009du, 0x000200f8u, 0x0000009cu, 0x0004003du,
    0x00000006u, 0x000000a2u, 0x0000009au, 0x0004003du, 0x00000006u, 0x000000a3u,
    0x0000007cu, 0x0004003du, 0x00000006u, 0x000000a4u, 0x0000009au, 0x00060041u,
    0x0000001eu, 0x000000a5u, 0x00000034u, 0x000000a3u, 0x000000a4u, 0x0004003du,
    0x00000012u, 0x000000a6u, 0x000000a5u, 0x0004003du, 0x00000006u, 0x000000a7u,
    0x00000081u, 0x0004003du, 0x00000006u, 0x000000a8u, 0x0000009au, 0x00060041u,
    0x0000001eu, 0x000000a9u, 0x00000034u, 0x000000a7u, 0x000000a8u, 0x0004003du,
    0x00000012u, 0x000000aau, 0x000000a9u, 0x00050081u, 0x00000012u, 0x000000abu,
    0x000000a6u, 0x000000aau, 0x0004003du, 0x00000006u, 0x000000acu, 0x0000006bu,
    0x0004003du, 0x00000006u, 0x000000adu, 0x0000009au, 0x00060041u, 0x0000001eu,
    0x000000aeu, 0x00000034u, 0x000000acu, 0x000000adu, 0x0004003du, 0x00000012u,
    0x000000afu, 0x000000aeu, 0x00050083u, 0x00000012u, 0x000000b0u, 0x000000abu,
    0x000000afu, 0x00050041u, 0x00000021u, 0x000000b1u, 0x00000030u, 0x000000a2u,
    0x0003003eu, 0x000000b1u, 0x000000b0u, 0x000200f9u, 0x0000009eu, 0x000200f8u,
    0x0000009eu, 0x0004003du, 0x00000006u, 0x000000b2u, 0x0000009au, 0x00050080u,
    0x00000006u, 0x000000b3u, 0x000000b2u, 0x0000003bu, 0x0003003eu, 0x0000009au,
    0x000000b3u, 0x000200f9u, 0x0000009bu, 0x000200f8u, 0x0000009du, 0x000100dau,
    0x0004003du, 0x00000006u, 0x000000b5u, 0x00000081u, 0x0003003eu, 0x000000b4u,
    0x000000b5u, 0x00050039u, 0x00000002u, 0x000000b6u, 0x0000000au, 0x000000b4u,
    0x000100dbu, 0x000100fdu, 0x00010038u, 0x00050036u, 0x00000002u, 0x0000000au,
    0x00000000u, 0x00000008u, 0x00030037u, 0x00000007u, 0x00000009u, 0x000200f8u,
    0x0000000bu, 0x0004003bu, 0x00000007u, 0x00000023u, 0x00000007u, 0x0004003du,
    0x00000006u, 0x0000001du, 0x00000009u, 0x00060041u, 0x0000001eu, 0x0000001fu,
    0x0000001cu, 0x0000001du, 0x00000016u, 0x0004003du, 0x00000012u, 0x00000020u,
    0x0000001fu, 0x00050041u, 0x00000021u, 0x00000022u, 0x00000015u, 0x00000016u,
    0x0003003eu, 0x00000022u, 0x00000020u, 0x0003003eu, 0x00000023u, 0x00000016u,
    0x000200f9u, 0x00000024u, 0x000200f8u, 0x00000024u, 0x000400f6u, 0x00000026u,
    0x00000027u, 0x00000000u, 0x000200f9u, 0x00000028u, 0x000200f8u, 0x00000028u,
    0x0004003du, 0x00000006u, 0x00000029u, 0x00000023u, 0x000500b1u, 0x0000002bu,
    0x0000002cu, 0x00000029u, 0x0000002au, 0x000400fau, 0x0000002cu, 0x00000025u,
    0x00000026u, 0x000200f8u, 0x00000025u, 0x0004003du, 0x00000006u, 0x00000031u,
    0x00000023u, 0x0004003du, 0x00000006u, 0x00000035u, 0x00000009u, 0x0004003du,
    0x00000006u, 0x00000036u, 0x00000023u, 0x00060041u, 0x0000001eu, 0x00000037u,
    0x00000034u, 0x00000035u, 0x00000036u, 0x0004003du, 0x00000012u, 0x00000038u,
    0x00000037u, 0x00050041u, 0x00000021u, 0x00000039u, 0x00000030u, 0x00000031u,
    0x0003003eu, 0x00000039u, 0x00000038u, 0x000200f9u, 0x00000027u, 0x000200f8u,
    0x00000027u, 0x0004003du, 0x00000006u, 0x0000003au, 0x00000023u, 0x00050080u,
    0x00000006u, 0x0000003cu, 0x0000003au, 0x0000003bu, 0x0003003eu, 0x00000023u,
    0x0000003cu, 0x000200f9u, 0x00000024u, 0x000200f8u, 0x00000026u, 0x000100dau,
    0x000100fdu, 0x00010038u, 0x00050036u, 0x0000000du, 0x00000010u, 0x00000000u,
    0x0000000eu, 0x00030037u, 0x00000007u, 0x0000000fu, 0x000200f8u, 0x00000011u,
    0x0004003bu, 0x0000003du, 0x0000003eu, 0x00000007u, 0x0004003bu, 0x00000048u,
    0x00000049u, 0x00000007u, 0x0004003du, 0x00000006u, 0x0000003fu, 0x0000000fu,
    0x00060041u, 0x0000001eu, 0x00000040u, 0x0000001cu, 0x0000003fu, 0x00000016u,
    0x0004003du, 0x00000012u, 0x00000041u, 0x00000040u, 0x0003003eu, 0x0000003eu,
    0x00000041u, 0x00050041u, 0x00000042u, 0x00000043u, 0x0000003eu, 0x00000019u,
    0x0004003du, 0x0000000cu, 0x00000044u, 0x00000043u, 0x0006000cu, 0x0000000cu,
    0x00000045u, 0x00000001u, 0x00000004u, 0x00000044u, 0x000500bau, 0x0000002bu,
    0x00000047u, 0x00000045u, 0x00000046u, 0x000300f7u, 0x0000004bu, 0x00000000u,
    0x000400fau, 0x00000047u, 0x0000004au, 0x00000052u, 0x000200f8u, 0x0000004au,
    0x0004003du, 0x00000012u, 0x0000004cu, 0x0000003eu, 0x0007004fu, 0x0000000du,
    0x0000004du, 0x0000004cu, 0x0000004cu, 0x00000000u, 0x00000001u, 0x00050041u,
    0x00000042u, 0x0000004eu, 0x0000003eu, 0x00000019u, 0x0004003du, 0x0000000cu,
    0x0000004fu, 0x0000004eu, 0x00050050u, 0x0000000du, 0x00000050u, 0x0000004fu,
    0x0000004fu, 0x00050088u, 0x0000000du, 0x00000051u, 0x0000004du, 0x00000050u,
    0x0003003eu, 0x00000049u, 0x00000051u, 0x000200f9u, 0x0000004bu, 0x000200f8u,
    0x00000052u, 0x0004003du, 0x00000012u, 0x00000053u, 0x0000003eu, 0x0007004fu,
    0x0000000du, 0x00000054u, 0x00000053u, 0x00000053u, 0x00000000u, 0x00000001u,
    0x0003003eu, 0x00000049u, 0x00000054u, 0x000200f9u, 0x0000004bu, 0x000200f8u,
    0x0000004bu, 0x0004003du, 0x0000000du, 0x00000055u, 0x00000049u, 0x000200feu,
    0x00000055u, 0x00010038u,
};

static VkShaderModule module_of(const uint32_t *said, uint32_t length) {
    VkShaderModuleCreateInfo asking = {0};
    asking.sType = VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO;
    asking.codeSize = (size_t)length * 4;
    asking.pCode = said;

    VkShaderModule made = VK_NULL_HANDLE;
    if (vkCreateShaderModule(drawing.device, &asking, NULL, &made) != VK_SUCCESS) {
        return VK_NULL_HANDLE;
    }
    return made;
}

/* What a host draws for a shape a title asked for.
 *
 * Two of these are not the same thing on both sides and are worth saying so. A
 * rectangle is three corners on the console and the fourth is worked out from
 * them, which is done on the device after the vertex shader where a device can,
 * and where it cannot, half of it is missing. A quad is four corners, which a
 * fan of triangles covers exactly, so that one loses nothing.
 */
static VkPrimitiveTopology topology_of(uint32_t kind) {
    switch (kind) {
    case XENOLITH_SHAPE_POINTS:
        return VK_PRIMITIVE_TOPOLOGY_POINT_LIST;
    case XENOLITH_SHAPE_LINES:
        return VK_PRIMITIVE_TOPOLOGY_LINE_LIST;
    case XENOLITH_SHAPE_LINE_RUN:
        return VK_PRIMITIVE_TOPOLOGY_LINE_STRIP;
    case XENOLITH_SHAPE_TRIANGLE_FAN:
    case XENOLITH_SHAPE_QUADS:
        return VK_PRIMITIVE_TOPOLOGY_TRIANGLE_FAN;
    case XENOLITH_SHAPE_TRIANGLE_RUN:
        return VK_PRIMITIVE_TOPOLOGY_TRIANGLE_STRIP;
    /* A rectangle reaches a host as its three corners, a triangle the device
     * finishes into two. */
    case XENOLITH_SHAPE_RECTANGLES:
        return VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST;
    default:
        return VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST;
    }
}

/* What a pair of shaders was made into, building it the first time it is
 * asked for.
 *
 * Nothing is said about what the vertices are, because a title draws without
 * naming any: what a vertex is comes out of the shader from the number it was
 * given, so there is nothing for a device to fetch.
 */
static VkPipeline pipeline_for(uint32_t blending, uint32_t written, uint32_t how_far,
                               uint32_t cut_off, uint32_t cut_off_at, uint32_t vertex_sum,
                               const uint32_t *vertex, uint32_t vertex_length, uint32_t pixel_sum,
                               const uint32_t *pixel, uint32_t pixel_length, uint32_t kind) {
    /* The ways a place keeps a colour that hold it to between nought and
     * one, by the console's numbering: four bytes, the same as seen by eye, and
     * the two of ten bits read either way. */
    uint32_t colour_kept = (drawing_into_shape >> 18) & 0xfu;
    uint32_t kept_as_a_fraction =
        colour_kept == 0u || colour_kept == 1u || colour_kept == 2u || colour_kept == 10u;
    const uint32_t built_for[XENOLITH_PIPELINE_SAID] = {
        vertex_sum, pixel_sum,  kind,           blending,          written,
        how_far,    cut_off,    cut_off_at,     pixel_goes_into,   kept_as_a_fraction};
    uint32_t kept_at = where_a_pipeline_is_kept(built_for);
    if (kept_at < XENOLITH_PIPELINES && built[kept_at].taken) {
        return built[kept_at].pipeline;
    }

    if (!make_what_a_frame_goes_through()) {
        return VK_NULL_HANDLE;
    }

    VkShaderModule first = module_of(vertex, vertex_length);
    VkShaderModule second = module_of(pixel, pixel_length);
    if (first == VK_NULL_HANDLE || second == VK_NULL_HANDLE) {
        return VK_NULL_HANDLE;
    }

    VkPipelineShaderStageCreateInfo stages[3] = {{0}, {0}, {0}};
    stages[0].sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO;
    stages[0].stage = VK_SHADER_STAGE_VERTEX_BIT;
    stages[0].module = first;
    stages[0].pName = "main";
    stages[1].sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO;
    stages[1].stage = VK_SHADER_STAGE_FRAGMENT_BIT;
    stages[1].module = second;
    stages[1].pName = "main";
    uint32_t stages_used = 2;
    VkShaderModule third = VK_NULL_HANDLE;
    if (kind == XENOLITH_SHAPE_RECTANGLES && rectangles_finished_on_the_device) {
        third = module_of(the_rest_of_a_rectangle,
                          sizeof the_rest_of_a_rectangle / sizeof *the_rest_of_a_rectangle);
    }
    if (third != VK_NULL_HANDLE) {
        stages[2].sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO;
        stages[2].stage = VK_SHADER_STAGE_GEOMETRY_BIT;
        stages[2].module = third;
        stages[2].pName = "main";
        stages_used = 3;
    }

    /* The test on how solid a place has to be to be drawn, which a pixel shader
     * was written to make but not told the terms of. Settling it here rather
     * than when the shader was read is what lets one reading serve every test,
     * and a driver folds the test away where it draws everything. */
    /* Beside it, which register the shader finds where its pixel is in, and
     * whether what it puts out is held to a fraction of the whole. */
    uint32_t the_test[4] = {cut_off, cut_off_at, pixel_goes_into, kept_as_a_fraction};
    VkSpecializationMapEntry each_part[4] = {{0}, {0}, {0}, {0}};
    for (uint32_t one = 0; one < 4u; one++) {
        each_part[one].constantID = one;
        each_part[one].offset = one * (uint32_t)sizeof the_test[0];
        each_part[one].size = sizeof the_test[0];
    }

    VkSpecializationInfo settled = {0};
    settled.mapEntryCount = 4;
    settled.pMapEntries = each_part;
    settled.dataSize = sizeof the_test;
    settled.pData = the_test;
    /* A vertex shader carries no settled constants, so only the fragment stage
     * is told about them. */
    stages[1].pSpecializationInfo = &settled;

    VkPipelineVertexInputStateCreateInfo nothing_fetched = {0};
    nothing_fetched.sType = VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO;

    VkPipelineInputAssemblyStateCreateInfo shape = {0};
    shape.sType = VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO;
    shape.topology = topology_of(kind);
    /* A run of shapes in a title's list starts again at a word of all ones,
     * which a host only looks for where it is told to. A list of separate
     * shapes cannot be told to, and never holds one. */
    shape.primitiveRestartEnable = shape.topology == VK_PRIMITIVE_TOPOLOGY_LINE_STRIP
                                   || shape.topology == VK_PRIMITIVE_TOPOLOGY_TRIANGLE_STRIP
                                   || shape.topology == VK_PRIMITIVE_TOPOLOGY_TRIANGLE_FAN;

    VkViewport seen = {0};
    seen.width = (float)XENOLITH_GRID_ACROSS;
    seen.height = (float)XENOLITH_GRID_DOWN;
    seen.maxDepth = 1.0f;

    VkRect2D kept = {0};
    kept.extent.width = XENOLITH_GRID_ACROSS;
    kept.extent.height = XENOLITH_GRID_DOWN;

    VkPipelineViewportStateCreateInfo where = {0};
    where.sType = VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO;
    where.viewportCount = 1;
    where.pViewports = &seen;
    where.scissorCount = 1;
    where.pScissors = &kept;

    /* What a title keeps is said per draw rather than built into a pipeline.
     *
     * A title changes what it clips to between draws as readily as it changes
     * anything else, and one pipeline is made for each pair of shaders. Baking
     * it in would mean a pipeline for every window a title ever asks for, and
     * the wrong one for every draw that came after the window changed. */
    const VkDynamicState changes[] = {VK_DYNAMIC_STATE_SCISSOR, VK_DYNAMIC_STATE_VIEWPORT,
                                      VK_DYNAMIC_STATE_DEPTH_BIAS};
    VkPipelineDynamicStateCreateInfo said_per_draw = {0};
    said_per_draw.sType = VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO;
    said_per_draw.dynamicStateCount = 3;
    said_per_draw.pDynamicStates = changes;

    VkPipelineRasterizationStateCreateInfo filling = {0};
    filling.sType = VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO;
    filling.polygonMode = VK_POLYGON_MODE_FILL;
    filling.cullMode = VK_CULL_MODE_NONE;
    filling.frontFace = VK_FRONT_FACE_COUNTER_CLOCKWISE;
    filling.lineWidth = 1.0f;
    filling.depthBiasEnable = VK_TRUE;

    VkPipelineMultisampleStateCreateInfo once = {0};
    once.sType = VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO;
    once.rasterizationSamples = VK_SAMPLE_COUNT_1_BIT;

    VkPipelineColorBlendAttachmentState keeping = {0};
    keeping.colorWriteMask = what_is_written(written);
    if (worth_mixing(blending)) {
        keeping.blendEnable = VK_TRUE;
        keeping.srcColorBlendFactor = mixing_by(blending & 0x1fu);
        keeping.dstColorBlendFactor = mixing_by((blending >> 8) & 0x1fu);
        keeping.colorBlendOp = joining_by((blending >> 5) & 7u);
        keeping.srcAlphaBlendFactor = mixing_by((blending >> 16) & 0x1fu);
        keeping.dstAlphaBlendFactor = mixing_by((blending >> 24) & 0x1fu);
        keeping.alphaBlendOp = joining_by((blending >> 21) & 7u);
    }

    VkPipelineColorBlendStateCreateInfo mixing = {0};
    mixing.sType = VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO;
    mixing.attachmentCount = 1;
    mixing.pAttachments = &keeping;
    if (painting_over_everything()) {
        mixing.logicOpEnable = VK_TRUE;
        mixing.logicOp = VK_LOGIC_OP_SET;
    }

    /* Whether to look at how far away a place already is before drawing over
     * it, whether to say how far away this one is, and which way round the
     * comparison goes. The three sit together in the one register, and the
     * hardware numbers the comparisons the same way a host does. */
    VkPipelineDepthStencilStateCreateInfo how_near = {0};
    how_near.sType = VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO;
    how_near.depthTestEnable = ((how_far >> 1) & 1u) != 0 ? VK_TRUE : VK_FALSE;
    how_near.depthWriteEnable = ((how_far >> 2) & 1u) != 0 ? VK_TRUE : VK_FALSE;
    how_near.depthCompareOp = (VkCompareOp)((how_far >> 4) & 7u);
    how_near.maxDepthBounds = 1.0f;

    VkGraphicsPipelineCreateInfo asking = {0};
    asking.sType = VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO;
    asking.stageCount = stages_used;
    asking.pStages = stages;
    asking.pVertexInputState = &nothing_fetched;
    asking.pInputAssemblyState = &shape;
    asking.pViewportState = &where;
    asking.pRasterizationState = &filling;
    asking.pMultisampleState = &once;
    asking.pDepthStencilState = &how_near;
    asking.pColorBlendState = &mixing;
    asking.pDynamicState = &said_per_draw;
    asking.layout = through.layout;
    asking.renderPass = through.pass;

    VkPipeline made = VK_NULL_HANDLE;
    VkResult said = vkCreateGraphicsPipelines(drawing.device, VK_NULL_HANDLE, 1, &asking, NULL,
                                              &made);
    vkDestroyShaderModule(drawing.device, first, NULL);
    vkDestroyShaderModule(drawing.device, second, NULL);
    if (third != VK_NULL_HANDLE) {
        vkDestroyShaderModule(drawing.device, third, NULL);
    }

    if (said != VK_SUCCESS) {
        return VK_NULL_HANDLE;
    }

    if (kept_at < XENOLITH_PIPELINES) {
        built[kept_at].taken = 1;
        memcpy(built[kept_at].said, built_for, sizeof built[kept_at].said);
        built[kept_at].pipeline = made;
    }
    return made;
}

int xenolith_can_draw_with(uint32_t vertex_sum, const uint32_t *vertex, uint32_t vertex_length,
                           uint32_t pixel_sum, const uint32_t *pixel, uint32_t pixel_length) {
    /* Asking only whether a pair can be built at all, so it is asked about
     * without any mixing and writing every part of a colour: the new colour
     * whole and none of the old. */
    return pipeline_for(XENOLITH_NOT_MIXING, XENOLITH_WRITE_EVERY_PART, XENOLITH_HOW_FAR_IGNORED,
                        XENOLITH_DRAWN_HOWEVER_SOLID, 0, vertex_sum, vertex,
                        vertex_length, pixel_sum, pixel, pixel_length, XENOLITH_SHAPE_TRIANGLES)
           != VK_NULL_HANDLE;
}

/* Whether how far away things are starts again from nothing.
 *
 * A console keeps what it drew, and how far away it was, in memory beside the
 * hardware, and a title copies out of that many times in one frame. Each copy
 * says whether to wipe what it copied from, and it is the title that decides.
 * Wiping how far away everything is at every copy loses what a first pass over
 * the scene worked out, so a second pass that draws only where it agrees with
 * that draws nowhere at all, and the scene comes out empty.
 */
static int wipe_the_colours;
static uint32_t colours_start_at;

/* Whether a frame is being recorded, so that beginning one twice or drawing
 * into one that never began cannot happen. */
static int recording;


/* The last window and view a draw was given, said again when a lot of
 * commands is handed over in the middle of a frame and the frame carries on in
 * a new one, which starts knowing neither. */
static VkViewport last_seen;
static int seen_given;
static VkRect2D last_kept;
static int kept_given;

/* Wipes the surface as it stands, between frames, to the colour a title said. */
static int begin_the_batch(void);

static int wipe_the_surface_now(uint32_t to) {
    if (!begin_the_batch()) {
        return 0;
    }
    VkCommandBuffer one = surface.commands;
    VkClearColorValue cleared_to = {0};
    cleared_to.float32[0] = (float)((to >> 16) & 0xffu) / 255.0f;
    cleared_to.float32[1] = (float)((to >> 8) & 0xffu) / 255.0f;
    cleared_to.float32[2] = (float)(to & 0xffu) / 255.0f;
    cleared_to.float32[3] = (float)((to >> 24) & 0xffu) / 255.0f;
    VkImageSubresourceRange every_part = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1};
    a_picture_becomes(one, surface.image, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                      VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL);
    vkCmdClearColorImage(one, surface.image, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, &cleared_to,
                         1, &every_part);
    anything_recorded = 1;
    a_picture_becomes(one, surface.image, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                      VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL);
    return 1;
}

/* Says whether the copy just made asked for the colours to start again, and
 * what to start them as.
 *
 * They are wiped at once, while the surface still holds the place the copy was
 * made from. Left for the next frame instead, the wipe fell on whichever place
 * that frame drew into, and the place a title copied from kept what it meant
 * to start again: a frame drawn a part at a time began each part over the last
 * one, and what it told apart by how solid it was came out the wrong way. */
void xenolith_wipe_the_colours_after_a_copy(int wipe, uint32_t to) {
    if (!wipe) {
        return;
    }
    colours_start_at = to;
    pthread_mutex_lock(&one_drawing);
    if (!surface.read_out || recording || !wipe_the_surface_now(to)) {
        wipe_the_colours = 1;
    }
    pthread_mutex_unlock(&one_drawing);
}

static int wipe_how_far = 1;

/* What it starts again as.
 *
 * Not always the far end. A title says which way round it counts distance: one
 * that counts from the far end towards the eye starts everywhere at nothing and
 * keeps whatever is further, and a scene drawn that way against a buffer
 * started at the far end keeps nothing at all, which is a whole world of
 * geometry drawn and no pixel written. The title says what to start it as in
 * the register beside the one saying whether to. */
static float how_far_to_start_at = 1.0f;

void xenolith_wipe_how_far_after_a_copy(int wipe, float to) {
    if (wipe) {
        wipe_how_far = 1;
        how_far_to_start_at = to;
    }
}

/* Starts a frame, clearing it to the colour a title starts from.
 *
 * That colour is black, and black here means nothing was drawn rather than any
 * value a title chose: what a title clears to is kept in a shader constant this
 * does not read yet, and picking one would be inventing what it said.
 */
/* Rounds a place in an arena up to one a device will take an offset at. */
static VkDeviceSize lined_up(VkDeviceSize at, VkDeviceSize on) {
    return (at + on - 1u) / on * on;
}

/* Says once that a frame drew more than a frame's room holds.
 *
 * Said rather than passed over, and the draw that asked is refused rather than
 * given room from the start of the arena. Wrapping around would have it read
 * what an earlier draw in the same frame wrote, which is a scene that is wrong
 * in a way nothing reports: the geometry is there, the vertices are somebody
 * else's, and what appears is a shape nobody asked for. A draw that does not
 * happen is a gap in a scene that can be seen. */
static int said_what_a_frame_cannot_hold;

static void more_than_a_frame_holds(const char *what) {
    if (said_what_a_frame_cannot_hold) {
        return;
    }
    said_what_a_frame_cannot_hold = 1;
    fprintf(stderr,
            "xenolith: a frame asked for more %s than a frame is given room for, so the draws "
            "that did not fit are left out rather than given room they do not have\n",
            what);
}

uint32_t *xenolith_room_for_vertices(uint32_t *how_many) {
    if (!make_what_a_frame_goes_through() || how_many == NULL) {
        return NULL;
    }

    if (through.vertex_at + XENOLITH_VERTEX_BYTES
        > (VkDeviceSize)(current_lot + 1u) * XENOLITH_VERTEX_ROOM) {
        more_than_a_frame_holds("vertices");
        /* Nothing, so the draw that asked for this reads nothing and is left
         * out. A caller given nothing does not draw. */
        through.vertex_from = 0;
        through.vertex_wide = 0;
        *how_many = 0;
        return NULL;
    }

    through.vertex_from = through.vertex_at;
    through.vertex_wide = XENOLITH_VERTEX_BYTES;
    through.numbers = 0;
    through.first_number = 0;
    *how_many = XENOLITH_VERTEX_WORDS;
    return through.vertex_words + through.vertex_from / 4u;
}

uint32_t xenolith_where_the_room_begins(uint32_t *given_out) {
    if (given_out != NULL) {
        *given_out = room_given_out_again;
    }
    return (uint32_t)((through.vertex_from - (VkDeviceSize)current_lot * XENOLITH_VERTEX_ROOM)
                      / 4u);
}

void xenolith_streams_begin_at(const uint32_t *where) {
    if (!through.made || where == NULL) {
        return;
    }
    uint32_t *words = (uint32_t *)(through.constant_bytes + through.constant_from);
    memcpy(&words[XENOLITH_THE_STARTS * 4u], where, XENOLITH_VERTEX_SLOTS * sizeof *where);
}

void xenolith_done_with_vertices(uint32_t words) {
    if (!through.made) {
        return;
    }
    VkDeviceSize wide = (VkDeviceSize)words * 4u;
    if (wide == 0 || wide > XENOLITH_VERTEX_BYTES) {
        wide = wide == 0 ? through.offsets_line_up_on : XENOLITH_VERTEX_BYTES;
    }
    through.vertex_wide = lined_up(wide, 4u);
    through.vertex_at = lined_up(through.vertex_from + through.vertex_wide,
                                 through.offsets_line_up_on);
}

void xenolith_number_the_vertices(uint32_t at, uint32_t count, uint32_t first) {
    through.numbers_at = at;
    through.numbers = count;
    through.first_number = first;
}

void xenolith_push_away_by(float outright, float along_a_slope) {
    through.pushed_outright = outright;
    through.pushed_along_a_slope = along_a_slope;
}

/* What the hardware would have done to a position, as this run was told it. */
static struct {
    float scale[4];
    float offset[4];
    int given;
} moving;

void xenolith_move_a_position_by(const float *scale, const float *offset) {
    if (scale == NULL || offset == NULL) {
        moving.given = 0;
        return;
    }
    for (unsigned int at = 0; at < 4u; at++) {
        moving.scale[at] = scale[at];
        moving.offset[at] = offset[at];
    }
    moving.given = 1;
}

int xenolith_hand_over_constants(const uint8_t *from, const uint8_t *truths,
                                 const uint8_t *counts) {
    if (!make_what_a_frame_goes_through() || from == NULL) {
        return 0;
    }

    /* Room for this draw's constants, its vertices and its saying, handed over
     * early where what has been recorded has used it up, so that nothing a draw
     * reads is given out again before the draw has run. */
    if (through.constant_at + XENOLITH_CONSTANT_BYTES
            > (VkDeviceSize)(current_lot + 1u) * XENOLITH_CONSTANT_ROOM
        || through.vertex_at + XENOLITH_VERTEX_BYTES
               > (VkDeviceSize)(current_lot + 1u) * XENOLITH_VERTEX_ROOM
        || sayings_taken + 1u >= XENOLITH_SAYINGS) {
        pthread_mutex_lock(&one_drawing);
        send_what_was_recorded();
        pthread_mutex_unlock(&one_drawing);
    }
    if (through.constant_at + XENOLITH_CONSTANT_BYTES
        > (VkDeviceSize)(current_lot + 1u) * XENOLITH_CONSTANT_ROOM) {
        more_than_a_frame_holds("shader constants");
        /* Refused rather than wrapped: a shader handed the room at the front of
         * the arena would read another draw's constants, which is a colour and
         * a transform nothing asked for. */
        through.constant_from = 0;
        through.constant_at = 0;
        return 0;
    }
    through.constant_from = through.constant_at;
    through.constant_at =
        lined_up(through.constant_from + XENOLITH_CONSTANT_BYTES, through.offsets_line_up_on);

    /* The console keeps these the other way round from a host, so each word is
     * turned as it is copied. Four of them make one constant. */
    uint32_t *words = (uint32_t *)(through.constant_bytes + through.constant_from);
    for (uint32_t at = 0; at < XENOLITH_ALL_THE_CONSTANTS * 4u; at++) {
        uint32_t was = ((uint32_t)from[at * 4] << 24) | ((uint32_t)from[at * 4 + 1] << 16)
                       | ((uint32_t)from[at * 4 + 2] << 8) | (uint32_t)from[at * 4 + 3];
        words[at] = was;
    }

    float *beside = (float *)&words[XENOLITH_ALL_THE_CONSTANTS * 4u];
    for (unsigned int at = 0; at < 8u; at++) {
        beside[at] = at < 4u ? 1.0f : 0.0f;
    }
    if (moving.given) {
        beside[0] = moving.scale[0];
        beside[1] = moving.scale[1];
        beside[2] = moving.scale[2];
        beside[3] = moving.scale[3];
        beside[4] = moving.offset[0];
        beside[5] = moving.offset[1];
        beside[6] = moving.offset[2];
        beside[7] = moving.offset[3];
    }

    /* The truths, kept as they are rather than as numbers, because what a
     * shader does with one is look at a single bit of it. */
    uint32_t *asked = &words[(XENOLITH_ALL_THE_CONSTANTS + 2u) * 4u];
    for (unsigned int at = 0; at < 8u; at++) {
        asked[at] = truths == NULL
                        ? 0u
                        : (((uint32_t)truths[at * 4] << 24) | ((uint32_t)truths[at * 4 + 1] << 16)
                           | ((uint32_t)truths[at * 4 + 2] << 8) | (uint32_t)truths[at * 4 + 3]);
    }

    /* The counts, kept as they are for the same reason: what a shader wants of
     * one is the handful of bits saying how many times to go round. */
    uint32_t *rounds = &words[(XENOLITH_ALL_THE_CONSTANTS + 4u) * 4u];
    for (unsigned int at = 0; at < 32u; at++) {
        rounds[at] = counts == NULL
                         ? 0u
                         : (((uint32_t)counts[at * 4] << 24) | ((uint32_t)counts[at * 4 + 1] << 16)
                            | ((uint32_t)counts[at * 4 + 2] << 8) | (uint32_t)counts[at * 4 + 3]);
    }

    return 1;
}

/* Somewhere to hold the surface, or how far away each place is, while the
 * samples in it are moved round. */
static struct {
    VkImage image;
    VkDeviceMemory memory;
} moving_round[2];

static VkImage somewhere_to_move_samples_round(int how_far) {
    if (moving_round[how_far].image != VK_NULL_HANDLE) {
        return moving_round[how_far].image;
    }
    VkImageCreateInfo wanted = {0};
    wanted.sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO;
    wanted.imageType = VK_IMAGE_TYPE_2D;
    wanted.format = how_far ? XENOLITH_HOW_FAR_FORMAT : XENOLITH_SURFACE_FORMAT;
    wanted.extent.width = XENOLITH_GRID_ACROSS;
    wanted.extent.height = XENOLITH_GRID_DOWN;
    wanted.extent.depth = 1;
    wanted.mipLevels = 1;
    wanted.arrayLayers = 1;
    wanted.samples = VK_SAMPLE_COUNT_1_BIT;
    wanted.tiling = VK_IMAGE_TILING_OPTIMAL;
    wanted.usage = VK_IMAGE_USAGE_TRANSFER_SRC_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT
                   | VK_IMAGE_USAGE_SAMPLED_BIT;
    wanted.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
    VkImage made = VK_NULL_HANDLE;
    if (vkCreateImage(drawing.device, &wanted, NULL, &made) != VK_SUCCESS) {
        return VK_NULL_HANDLE;
    }
    VkMemoryRequirements needs;
    vkGetImageMemoryRequirements(drawing.device, made, &needs);
    VkMemoryAllocateInfo asking = {0};
    asking.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
    asking.allocationSize = needs.size;
    asking.memoryTypeIndex =
        memory_that_suits(needs.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);
    VkDeviceMemory its_own = VK_NULL_HANDLE;
    if (vkAllocateMemory(drawing.device, &asking, NULL, &its_own) != VK_SUCCESS) {
        vkDestroyImage(drawing.device, made, NULL);
        return VK_NULL_HANDLE;
    }
    if (vkBindImageMemory(drawing.device, made, its_own, 0) != VK_SUCCESS) {
        vkFreeMemory(drawing.device, its_own, NULL);
        vkDestroyImage(drawing.device, made, NULL);
        return VK_NULL_HANDLE;
    }
    moving_round[how_far].image = made;
    moving_round[how_far].memory = its_own;
    return made;
}

/* Says an image moved round is to be used next for something else, waiting for
 * whatever touched it last however it touched it. */
static void samples_become(VkCommandBuffer on, VkImage which, VkImageAspectFlags aspect,
                           VkImageLayout was, VkImageLayout now) {
    VkImageMemoryBarrier turning = {0};
    turning.sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER;
    turning.oldLayout = was;
    turning.newLayout = now;
    turning.srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
    turning.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
    turning.image = which;
    turning.subresourceRange.aspectMask = aspect;
    turning.subresourceRange.levelCount = 1;
    turning.subresourceRange.layerCount = 1;
    turning.srcAccessMask = VK_ACCESS_TRANSFER_READ_BIT | VK_ACCESS_TRANSFER_WRITE_BIT
                            | VK_ACCESS_SHADER_READ_BIT;
    turning.dstAccessMask = VK_ACCESS_TRANSFER_READ_BIT | VK_ACCESS_TRANSFER_WRITE_BIT
                            | VK_ACCESS_SHADER_READ_BIT;
    vkCmdPipelineBarrier(on, VK_PIPELINE_STAGE_ALL_COMMANDS_BIT,
                         VK_PIPELINE_STAGE_ALL_COMMANDS_BIT, 0, 0, NULL, 0, NULL, 1, &turning);
}


/* Shaders of the runtime's own. One covers the whole of what is drawn into with
 * a single triangle. Two move the samples of a place round in one draw, each
 * putting where a pixel is the sample read from the place the console keeps
 * it, a colour or how far away it is: they swap the second and third of every
 * four across, down, or both, as the two numbers handed to them say, which is
 * exchanging the lowest two bits of where the pixel is. The last turns how far
 * away things are into the bytes a copy of it writes, three of distance over
 * one of the stencil, counted as a fraction of the whole or as the console's
 * small float as it is told, for a title that reads those bytes back as a
 * picture. Made once, outside the build, from a few lines each. */
static const uint32_t a_triangle_over_everything[] = {
    0x07230203u, 0x00010000u, 0x0008000bu, 0x0000002cu, 0x00000000u, 0x00020011u,
    0x00000001u, 0x0006000bu, 0x00000001u, 0x4c534c47u, 0x6474732eu, 0x3035342eu,
    0x00000000u, 0x0003000eu, 0x00000000u, 0x00000001u, 0x0007000fu, 0x00000000u,
    0x00000004u, 0x6e69616du, 0x00000000u, 0x0000000cu, 0x0000001du, 0x00040047u,
    0x0000000cu, 0x0000000bu, 0x0000002au, 0x00030047u, 0x0000001bu, 0x00000002u,
    0x00050048u, 0x0000001bu, 0x00000000u, 0x0000000bu, 0x00000000u, 0x00050048u,
    0x0000001bu, 0x00000001u, 0x0000000bu, 0x00000001u, 0x00050048u, 0x0000001bu,
    0x00000002u, 0x0000000bu, 0x00000003u, 0x00050048u, 0x0000001bu, 0x00000003u,
    0x0000000bu, 0x00000004u, 0x00020013u, 0x00000002u, 0x00030021u, 0x00000003u,
    0x00000002u, 0x00030016u, 0x00000006u, 0x00000020u, 0x00040017u, 0x00000007u,
    0x00000006u, 0x00000002u, 0x00040015u, 0x0000000au, 0x00000020u, 0x00000001u,
    0x00040020u, 0x0000000bu, 0x00000001u, 0x0000000au, 0x0004003bu, 0x0000000bu,
    0x0000000cu, 0x00000001u, 0x0004002bu, 0x0000000au, 0x0000000eu, 0x00000001u,
    0x0004002bu, 0x0000000au, 0x00000010u, 0x00000002u, 0x00040017u, 0x00000017u,
    0x00000006u, 0x00000004u, 0x00040015u, 0x00000018u, 0x00000020u, 0x00000000u,
    0x0004002bu, 0x00000018u, 0x00000019u, 0x00000001u, 0x0004001cu, 0x0000001au,
    0x00000006u, 0x00000019u, 0x0006001eu, 0x0000001bu, 0x00000017u, 0x00000006u,
    0x0000001au, 0x0000001au, 0x00040020u, 0x0000001cu, 0x00000003u, 0x0000001bu,
    0x0004003bu, 0x0000001cu, 0x0000001du, 0x00000003u, 0x0004002bu, 0x0000000au,
    0x0000001eu, 0x00000000u, 0x0004002bu, 0x00000006u, 0x00000020u, 0x40000000u,
    0x0004002bu, 0x00000006u, 0x00000022u, 0x3f800000u, 0x0004002bu, 0x00000006u,
    0x00000025u, 0x00000000u, 0x00040020u, 0x00000029u, 0x00000003u, 0x00000017u,
    0x0005002cu, 0x00000007u, 0x0000002bu, 0x00000022u, 0x00000022u, 0x00050036u,
    0x00000002u, 0x00000004u, 0x00000000u, 0x00000003u, 0x000200f8u, 0x00000005u,
    0x0004003du, 0x0000000au, 0x0000000du, 0x0000000cu, 0x000500c4u, 0x0000000au,
    0x0000000fu, 0x0000000du, 0x0000000eu, 0x000500c7u, 0x0000000au, 0x00000011u,
    0x0000000fu, 0x00000010u, 0x0004006fu, 0x00000006u, 0x00000012u, 0x00000011u,
    0x000500c7u, 0x0000000au, 0x00000014u, 0x0000000du, 0x00000010u, 0x0004006fu,
    0x00000006u, 0x00000015u, 0x00000014u, 0x00050050u, 0x00000007u, 0x00000016u,
    0x00000012u, 0x00000015u, 0x0005008eu, 0x00000007u, 0x00000021u, 0x00000016u,
    0x00000020u, 0x00050083u, 0x00000007u, 0x00000024u, 0x00000021u, 0x0000002bu,
    0x00050051u, 0x00000006u, 0x00000026u, 0x00000024u, 0x00000000u, 0x00050051u,
    0x00000006u, 0x00000027u, 0x00000024u, 0x00000001u, 0x00070050u, 0x00000017u,
    0x00000028u, 0x00000026u, 0x00000027u, 0x00000025u, 0x00000022u, 0x00050041u,
    0x00000029u, 0x0000002au, 0x0000001du, 0x0000001eu, 0x0003003eu, 0x0000002au,
    0x00000028u, 0x000100fdu, 0x00010038u,
};

static const uint32_t moving_colours_round[] = {
    0x07230203u, 0x00010000u, 0x0008000bu, 0x0000003du, 0x00000000u, 0x00020011u,
    0x00000001u, 0x0006000bu, 0x00000001u, 0x4c534c47u, 0x6474732eu, 0x3035342eu,
    0x00000000u, 0x0003000eu, 0x00000000u, 0x00000001u, 0x0007000fu, 0x00000004u,
    0x00000004u, 0x6e69616du, 0x00000000u, 0x0000000du, 0x00000025u, 0x00030010u,
    0x00000004u, 0x00000007u, 0x00040047u, 0x0000000du, 0x0000000bu, 0x0000000fu,
    0x00040047u, 0x00000025u, 0x0000001eu, 0x00000000u, 0x00040047u, 0x00000029u,
    0x00000021u, 0x00000000u, 0x00040047u, 0x00000029u, 0x00000022u, 0x00000000u,
    0x00030047u, 0x0000002du, 0x00000002u, 0x00050048u, 0x0000002du, 0x00000000u,
    0x00000023u, 0x00000000u, 0x00020013u, 0x00000002u, 0x00030021u, 0x00000003u,
    0x00000002u, 0x00040015u, 0x00000006u, 0x00000020u, 0x00000001u, 0x00040017u,
    0x00000007u, 0x00000006u, 0x00000002u, 0x00030016u, 0x0000000au, 0x00000020u,
    0x00040017u, 0x0000000bu, 0x0000000au, 0x00000004u, 0x00040020u, 0x0000000cu,
    0x00000001u, 0x0000000bu, 0x0004003bu, 0x0000000cu, 0x0000000du, 0x00000001u,
    0x00040017u, 0x0000000eu, 0x0000000au, 0x00000002u, 0x0004002bu, 0x00000006u,
    0x00000014u, 0xfffffffcu, 0x0004002bu, 0x00000006u, 0x00000018u, 0x00000001u,
    0x00040020u, 0x00000024u, 0x00000003u, 0x0000000bu, 0x0004003bu, 0x00000024u,
    0x00000025u, 0x00000003u, 0x00090019u, 0x00000026u, 0x0000000au, 0x00000001u,
    0x00000000u, 0x00000000u, 0x00000000u, 0x00000001u, 0x00000000u, 0x0003001bu,
    0x00000027u, 0x00000026u, 0x00040020u, 0x00000028u, 0x00000000u, 0x00000027u,
    0x0004003bu, 0x00000028u, 0x00000029u, 0x00000000u, 0x0003001eu, 0x0000002du,
    0x00000007u, 0x00040020u, 0x0000002eu, 0x00000009u, 0x0000002du, 0x0004003bu,
    0x0000002eu, 0x0000002fu, 0x00000009u, 0x0004002bu, 0x00000006u, 0x00000030u,
    0x00000000u, 0x00040020u, 0x00000031u, 0x00000009u, 0x00000007u, 0x0005002cu,
    0x00000007u, 0x00000034u, 0x00000030u, 0x00000030u, 0x00020014u, 0x00000035u,
    0x00040017u, 0x00000036u, 0x00000035u, 0x00000002u, 0x0005002cu, 0x00000007u,
    0x0000003bu, 0x00000014u, 0x00000014u, 0x0005002cu, 0x00000007u, 0x0000003cu,
    0x00000018u, 0x00000018u, 0x00050036u, 0x00000002u, 0x00000004u, 0x00000000u,
    0x00000003u, 0x000200f8u, 0x00000005u, 0x0004003du, 0x0000000bu, 0x0000000fu,
    0x0000000du, 0x0007004fu, 0x0000000eu, 0x00000010u, 0x0000000fu, 0x0000000fu,
    0x00000000u, 0x00000001u, 0x0004006eu, 0x00000007u, 0x00000011u, 0x00000010u,
    0x000500c7u, 0x00000007u, 0x00000016u, 0x00000011u, 0x0000003bu, 0x000500c7u,
    0x00000007u, 0x0000001au, 0x00000011u, 0x0000003cu, 0x000500c4u, 0x00000007u,
    0x0000001cu, 0x0000001au, 0x0000003cu, 0x000500c5u, 0x00000007u, 0x0000001du,
    0x00000016u, 0x0000001cu, 0x000500c3u, 0x00000007u, 0x00000020u, 0x00000011u,
    0x0000003cu, 0x000500c7u, 0x00000007u, 0x00000022u, 0x00000020u, 0x0000003cu,
    0x000500c5u, 0x00000007u, 0x00000023u, 0x0000001du, 0x00000022u, 0x0004003du,
    0x00000027u, 0x0000002au, 0x00000029u, 0x00050041u, 0x00000031u, 0x00000032u,
    0x0000002fu, 0x00000030u, 0x0004003du, 0x00000007u, 0x00000033u, 0x00000032u,
    0x000500abu, 0x00000036u, 0x00000037u, 0x00000033u, 0x00000034u, 0x000600a9u,
    0x00000007u, 0x00000038u, 0x00000037u, 0x00000023u, 0x00000011u, 0x00040064u,
    0x00000026u, 0x00000039u, 0x0000002au, 0x0007005fu, 0x0000000bu, 0x0000003au,
    0x00000039u, 0x00000038u, 0x00000002u, 0x00000030u, 0x0003003eu, 0x00000025u,
    0x0000003au, 0x000100fdu, 0x00010038u,
};

static const uint32_t distances_into_bytes[] = {
    0x07230203u, 0x00010000u, 0x0008000bu, 0x000000d5u, 0x00000000u, 0x00020011u,
    0x00000001u, 0x0006000bu, 0x00000001u, 0x4c534c47u, 0x6474732eu, 0x3035342eu,
    0x00000000u, 0x0003000eu, 0x00000000u, 0x00000001u, 0x0007000fu, 0x00000004u,
    0x00000004u, 0x6e69616du, 0x00000000u, 0x00000053u, 0x00000083u, 0x00030010u,
    0x00000004u, 0x00000007u, 0x00040047u, 0x0000004fu, 0x00000021u, 0x00000000u,
    0x00040047u, 0x0000004fu, 0x00000022u, 0x00000000u, 0x00040047u, 0x00000053u,
    0x0000000bu, 0x0000000fu, 0x00030047u, 0x00000059u, 0x00000002u, 0x00050048u,
    0x00000059u, 0x00000000u, 0x00000023u, 0x00000000u, 0x00050048u, 0x00000059u,
    0x00000001u, 0x00000023u, 0x00000008u, 0x00040047u, 0x00000083u, 0x0000001eu,
    0x00000000u, 0x00020013u, 0x00000002u, 0x00030021u, 0x00000003u, 0x00000002u,
    0x00030016u, 0x00000006u, 0x00000020u, 0x00040015u, 0x00000008u, 0x00000020u,
    0x00000000u, 0x0004002bu, 0x00000006u, 0x0000000eu, 0x00000000u, 0x00020014u,
    0x0000000fu, 0x0004002bu, 0x00000008u, 0x00000014u, 0x00000000u, 0x00040015u,
    0x00000018u, 0x00000020u, 0x00000001u, 0x0004001eu, 0x0000001bu, 0x00000006u,
    0x00000018u, 0x0004002bu, 0x00000018u, 0x00000021u, 0x00000001u, 0x0004002bu,
    0x00000018u, 0x00000023u, 0x0000000fu, 0x0004002bu, 0x00000018u, 0x00000026u,
    0x00000000u, 0x0004002bu, 0x00000018u, 0x0000002bu, 0x0000000eu, 0x0004002bu,
    0x00000006u, 0x0000002du, 0x49800000u, 0x0004002bu, 0x00000008u, 0x00000030u,
    0x000fffffu, 0x0004002bu, 0x00000008u, 0x00000037u, 0x00ffffffu, 0x0004002bu,
    0x00000006u, 0x0000003cu, 0x40000000u, 0x0004002bu, 0x00000006u, 0x0000003eu,
    0x3f800000u, 0x0004002bu, 0x00000018u, 0x00000045u, 0x00000014u, 0x00090019u,
    0x0000004cu, 0x00000006u, 0x00000001u, 0x00000000u, 0x00000000u, 0x00000000u,
    0x00000001u, 0x00000000u, 0x0003001bu, 0x0000004du, 0x0000004cu, 0x00040020u,
    0x0000004eu, 0x00000000u, 0x0000004du, 0x0004003bu, 0x0000004eu, 0x0000004fu,
    0x00000000u, 0x00040017u, 0x00000051u, 0x00000006u, 0x00000004u, 0x00040020u,
    0x00000052u, 0x00000001u, 0x00000051u, 0x0004003bu, 0x00000052u, 0x00000053u,
    0x00000001u, 0x00040017u, 0x00000054u, 0x00000006u, 0x00000002u, 0x00040017u,
    0x00000057u, 0x00000018u, 0x00000002u, 0x0004001eu, 0x00000059u, 0x00000057u,
    0x00000018u, 0x00040020u, 0x0000005au, 0x00000009u, 0x00000059u, 0x0004003bu,
    0x0000005au, 0x0000005bu, 0x00000009u, 0x00040020u, 0x0000005cu, 0x00000009u,
    0x00000057u, 0x00040020u, 0x00000063u, 0x00000009u, 0x00000018u, 0x0004002bu,
    0x00000006u, 0x00000070u, 0x4b7fffffu, 0x00040020u, 0x00000082u, 0x00000003u,
    0x00000051u, 0x0004003bu, 0x00000082u, 0x00000083u, 0x00000003u, 0x0004002bu,
    0x00000008u, 0x00000085u, 0x000000ffu, 0x0004002bu, 0x00000018u, 0x00000089u,
    0x00000008u, 0x0004002bu, 0x00000018u, 0x0000008eu, 0x00000010u, 0x0004002bu,
    0x00000006u, 0x000000d3u, 0x3b808081u, 0x0007002cu, 0x00000051u, 0x000000d4u,
    0x000000d3u, 0x000000d3u, 0x000000d3u, 0x000000d3u, 0x00050036u, 0x00000002u,
    0x00000004u, 0x00000000u, 0x00000003u, 0x000200f8u, 0x00000005u, 0x0004003du,
    0x0000004du, 0x00000050u, 0x0000004fu, 0x0004003du, 0x00000051u, 0x00000055u,
    0x00000053u, 0x0007004fu, 0x00000054u, 0x00000056u, 0x00000055u, 0x00000055u,
    0x00000000u, 0x00000001u, 0x0004006eu, 0x00000057u, 0x00000058u, 0x00000056u,
    0x00050041u, 0x0000005cu, 0x0000005du, 0x0000005bu, 0x00000026u, 0x0004003du,
    0x00000057u, 0x0000005eu, 0x0000005du, 0x00050084u, 0x00000057u, 0x0000005fu,
    0x00000058u, 0x0000005eu, 0x00040064u, 0x0000004cu, 0x00000060u, 0x00000050u,
    0x0007005fu, 0x00000051u, 0x00000061u, 0x00000060u, 0x0000005fu, 0x00000002u,
    0x00000026u, 0x00050051u, 0x00000006u, 0x00000062u, 0x00000061u, 0x00000000u,
    0x00050041u, 0x00000063u, 0x00000064u, 0x0000005bu, 0x00000021u, 0x0004003du,
    0x00000018u, 0x00000065u, 0x00000064u, 0x000500abu, 0x0000000fu, 0x00000066u,
    0x00000065u, 0x00000026u, 0x000300f7u, 0x00000068u, 0x00000000u, 0x000400fau,
    0x00000066u, 0x00000067u, 0x0000006du, 0x000200f8u, 0x00000067u, 0x000300f7u,
    0x000000cbu, 0x00000000u, 0x000300fbu, 0x00000014u, 0x000000a6u, 0x000200f8u,
    0x000000a6u, 0x000500bdu, 0x0000000fu, 0x000000a9u, 0x00000062u, 0x0000000eu,
    0x000300f7u, 0x000000abu, 0x00000000u, 0x000400fau, 0x000000a9u, 0x000000aau,
    0x000000abu, 0x000200f8u, 0x000000aau, 0x000200f9u, 0x000000cbu, 0x000200f8u,
    0x000000abu, 0x0006000cu, 0x0000001bu, 0x000000adu, 0x00000001u, 0x00000034u,
    0x00000062u, 0x00050051u, 0x00000018u, 0x000000aeu, 0x000000adu, 0x00000001u,
    0x00050051u, 0x00000006u, 0x000000afu, 0x000000adu, 0x00000000u, 0x00050080u,
    0x00000018u, 0x000000b2u, 0x000000aeu, 0x0000002bu, 0x000500b3u, 0x0000000fu,
    0x000000b4u, 0x000000b2u, 0x00000026u, 0x000300f7u, 0x000000bbu, 0x00000000u,
    0x000400fau, 0x000000b4u, 0x000000b5u, 0x000000bbu, 0x000200f8u, 0x000000b5u,
    0x0007000cu, 0x00000006u, 0x000000b7u, 0x00000001u, 0x00000035u, 0x00000062u,
    0x0000002bu, 0x00050085u, 0x00000006u, 0x000000b8u, 0x000000b7u, 0x0000002du,
    0x0004006du, 0x00000008u, 0x000000b9u, 0x000000b8u, 0x0007000cu, 0x00000008u,
    0x000000bau, 0x00000001u, 0x00000026u, 0x000000b9u, 0x00000030u, 0x000200f9u,
    0x000000cbu, 0x000200f8u, 0x000000bbu, 0x000500adu, 0x0000000fu, 0x000000bdu,
    0x000000b2u, 0x00000023u, 0x000300f7u, 0x000000bfu, 0x00000000u, 0x000400fau,
    0x000000bdu, 0x000000beu, 0x000000bfu, 0x000200f8u, 0x000000beu, 0x000200f9u,
    0x000000cbu, 0x000200f8u, 0x000000bfu, 0x00050085u, 0x00000006u, 0x000000c1u,
    0x000000afu, 0x0000003cu, 0x00050083u, 0x00000006u, 0x000000c2u, 0x000000c1u,
    0x0000003eu, 0x00050085u, 0x00000006u, 0x000000c3u, 0x000000c2u, 0x0000002du,
    0x0004006du, 0x00000008u, 0x000000c4u, 0x000000c3u, 0x0007000cu, 0x00000008u,
    0x000000c5u, 0x00000001u, 0x00000026u, 0x000000c4u, 0x00000030u, 0x0004007cu,
    0x00000008u, 0x000000c7u, 0x000000b2u, 0x000500c4u, 0x00000008u, 0x000000c8u,
    0x000000c7u, 0x00000045u, 0x000500c5u, 0x00000008u, 0x000000cau, 0x000000c8u,
    0x000000c5u, 0x000200f9u, 0x000000cbu, 0x000200f8u, 0x000000cbu, 0x000b00f5u,
    0x00000008u, 0x000000cfu, 0x00000014u, 0x000000aau, 0x000000bau, 0x000000b5u,
    0x00000037u, 0x000000beu, 0x000000cau, 0x000000bfu, 0x000200f9u, 0x00000068u,
    0x000200f8u, 0x0000006du, 0x00050085u, 0x00000006u, 0x00000071u, 0x00000062u,
    0x00000070u, 0x000500bcu, 0x0000000fu, 0x00000073u, 0x00000071u, 0x0000000eu,
    0x000300f7u, 0x00000076u, 0x00000000u, 0x000400fau, 0x00000073u, 0x00000075u,
    0x00000077u, 0x000200f8u, 0x00000075u, 0x000200f9u, 0x00000076u, 0x000200f8u,
    0x00000077u, 0x000500beu, 0x0000000fu, 0x00000079u, 0x00000071u, 0x00000070u,
    0x000300f7u, 0x0000007cu, 0x00000000u, 0x000400fau, 0x00000079u, 0x0000007bu,
    0x0000007du, 0x000200f8u, 0x0000007bu, 0x000200f9u, 0x0000007cu, 0x000200f8u,
    0x0000007du, 0x0004006du, 0x00000008u, 0x0000007fu, 0x00000071u, 0x000200f9u,
    0x0000007cu, 0x000200f8u, 0x0000007cu, 0x000700f5u, 0x00000008u, 0x000000cdu,
    0x00000037u, 0x0000007bu, 0x0000007fu, 0x0000007du, 0x000200f9u, 0x00000076u,
    0x000200f8u, 0x00000076u, 0x000700f5u, 0x00000008u, 0x000000ceu, 0x00000014u,
    0x00000075u, 0x000000cdu, 0x0000007cu, 0x000200f9u, 0x00000068u, 0x000200f8u,
    0x00000068u, 0x000700f5u, 0x00000008u, 0x000000d0u, 0x000000cfu, 0x000000cbu,
    0x000000ceu, 0x00000076u, 0x000500c7u, 0x00000008u, 0x00000086u, 0x000000d0u,
    0x00000085u, 0x00040070u, 0x00000006u, 0x00000087u, 0x00000086u, 0x000500c2u,
    0x00000008u, 0x0000008au, 0x000000d0u, 0x00000089u, 0x000500c7u, 0x00000008u,
    0x0000008bu, 0x0000008au, 0x00000085u, 0x00040070u, 0x00000006u, 0x0000008cu,
    0x0000008bu, 0x000500c2u, 0x00000008u, 0x0000008fu, 0x000000d0u, 0x0000008eu,
    0x000500c7u, 0x00000008u, 0x00000090u, 0x0000008fu, 0x00000085u, 0x00040070u,
    0x00000006u, 0x00000091u, 0x00000090u, 0x00070050u, 0x00000051u, 0x00000092u,
    0x0000000eu, 0x00000087u, 0x0000008cu, 0x00000091u, 0x00050085u, 0x00000051u,
    0x00000095u, 0x00000092u, 0x000000d4u, 0x0003003eu, 0x00000083u, 0x00000095u,
    0x000100fdu, 0x00010038u,
};

static const uint32_t moving_distances_round[] = {
    0x07230203u, 0x00010000u, 0x0008000bu, 0x00000040u, 0x00000000u, 0x00020011u,
    0x00000001u, 0x0006000bu, 0x00000001u, 0x4c534c47u, 0x6474732eu, 0x3035342eu,
    0x00000000u, 0x0003000eu, 0x00000000u, 0x00000001u, 0x0007000fu, 0x00000004u,
    0x00000004u, 0x6e69616du, 0x00000000u, 0x0000000du, 0x00000025u, 0x00030010u,
    0x00000004u, 0x00000007u, 0x00030010u, 0x00000004u, 0x0000000cu, 0x00040047u,
    0x0000000du, 0x0000000bu, 0x0000000fu, 0x00040047u, 0x00000025u, 0x0000000bu,
    0x00000016u, 0x00040047u, 0x00000029u, 0x00000021u, 0x00000000u, 0x00040047u,
    0x00000029u, 0x00000022u, 0x00000000u, 0x00030047u, 0x0000002du, 0x00000002u,
    0x00050048u, 0x0000002du, 0x00000000u, 0x00000023u, 0x00000000u, 0x00020013u,
    0x00000002u, 0x00030021u, 0x00000003u, 0x00000002u, 0x00040015u, 0x00000006u,
    0x00000020u, 0x00000001u, 0x00040017u, 0x00000007u, 0x00000006u, 0x00000002u,
    0x00030016u, 0x0000000au, 0x00000020u, 0x00040017u, 0x0000000bu, 0x0000000au,
    0x00000004u, 0x00040020u, 0x0000000cu, 0x00000001u, 0x0000000bu, 0x0004003bu,
    0x0000000cu, 0x0000000du, 0x00000001u, 0x00040017u, 0x0000000eu, 0x0000000au,
    0x00000002u, 0x0004002bu, 0x00000006u, 0x00000014u, 0xfffffffcu, 0x0004002bu,
    0x00000006u, 0x00000018u, 0x00000001u, 0x00040020u, 0x00000024u, 0x00000003u,
    0x0000000au, 0x0004003bu, 0x00000024u, 0x00000025u, 0x00000003u, 0x00090019u,
    0x00000026u, 0x0000000au, 0x00000001u, 0x00000000u, 0x00000000u, 0x00000000u,
    0x00000001u, 0x00000000u, 0x0003001bu, 0x00000027u, 0x00000026u, 0x00040020u,
    0x00000028u, 0x00000000u, 0x00000027u, 0x0004003bu, 0x00000028u, 0x00000029u,
    0x00000000u, 0x0003001eu, 0x0000002du, 0x00000007u, 0x00040020u, 0x0000002eu,
    0x00000009u, 0x0000002du, 0x0004003bu, 0x0000002eu, 0x0000002fu, 0x00000009u,
    0x0004002bu, 0x00000006u, 0x00000030u, 0x00000000u, 0x00040020u, 0x00000031u,
    0x00000009u, 0x00000007u, 0x0005002cu, 0x00000007u, 0x00000034u, 0x00000030u,
    0x00000030u, 0x00020014u, 0x00000035u, 0x00040017u, 0x00000036u, 0x00000035u,
    0x00000002u, 0x0005002cu, 0x00000007u, 0x0000003eu, 0x00000014u, 0x00000014u,
    0x0005002cu, 0x00000007u, 0x0000003fu, 0x00000018u, 0x00000018u, 0x00050036u,
    0x00000002u, 0x00000004u, 0x00000000u, 0x00000003u, 0x000200f8u, 0x00000005u,
    0x0004003du, 0x0000000bu, 0x0000000fu, 0x0000000du, 0x0007004fu, 0x0000000eu,
    0x00000010u, 0x0000000fu, 0x0000000fu, 0x00000000u, 0x00000001u, 0x0004006eu,
    0x00000007u, 0x00000011u, 0x00000010u, 0x000500c7u, 0x00000007u, 0x00000016u,
    0x00000011u, 0x0000003eu, 0x000500c7u, 0x00000007u, 0x0000001au, 0x00000011u,
    0x0000003fu, 0x000500c4u, 0x00000007u, 0x0000001cu, 0x0000001au, 0x0000003fu,
    0x000500c5u, 0x00000007u, 0x0000001du, 0x00000016u, 0x0000001cu, 0x000500c3u,
    0x00000007u, 0x00000020u, 0x00000011u, 0x0000003fu, 0x000500c7u, 0x00000007u,
    0x00000022u, 0x00000020u, 0x0000003fu, 0x000500c5u, 0x00000007u, 0x00000023u,
    0x0000001du, 0x00000022u, 0x0004003du, 0x00000027u, 0x0000002au, 0x00000029u,
    0x00050041u, 0x00000031u, 0x00000032u, 0x0000002fu, 0x00000030u, 0x0004003du,
    0x00000007u, 0x00000033u, 0x00000032u, 0x000500abu, 0x00000036u, 0x00000037u,
    0x00000033u, 0x00000034u, 0x000600a9u, 0x00000007u, 0x00000038u, 0x00000037u,
    0x00000023u, 0x00000011u, 0x00040064u, 0x00000026u, 0x00000039u, 0x0000002au,
    0x0007005fu, 0x0000000bu, 0x0000003au, 0x00000039u, 0x00000038u, 0x00000002u,
    0x00000030u, 0x00050051u, 0x0000000au, 0x0000003du, 0x0000003au, 0x00000000u,
    0x0003003eu, 0x00000025u, 0x0000003du, 0x000100fdu, 0x00010038u,
};

/* What moving samples round in a draw takes: the images to read from as
 * pictures, a pipeline for each, and what the frame being begun still has to
 * move round once its pass has begun. */
static struct {
    VkImageView seen[2];
    VkSampler way;
    VkDescriptorSetLayout shape;
    VkDescriptorPool holding;
    VkDescriptorSet said[2];
    VkPipelineLayout layout;
    VkPipeline pipeline[2];
    int made;
    int refused;
    int waiting[2];
    int32_t swapped[2][2];
    uint32_t wide[2];
    uint32_t tall[2];
} rearranging;

/* A pipeline drawing one triangle over the whole of what it draws into with a
 * shader of the runtime's own, writing colours or how far away things are. */
static VkPipeline a_pipeline_of_our_own(const uint32_t *pixel, uint32_t pixel_words,
                                        VkPipelineLayout layout, VkRenderPass pass,
                                        int how_far) {
    VkShaderModule first = module_of(a_triangle_over_everything,
                                     sizeof a_triangle_over_everything / 4u);
    VkShaderModule second = module_of(pixel, pixel_words);
    VkPipeline made = VK_NULL_HANDLE;
    if (first != VK_NULL_HANDLE && second != VK_NULL_HANDLE) {
        VkPipelineShaderStageCreateInfo stages[2] = {{0}, {0}};
        stages[0].sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO;
        stages[0].stage = VK_SHADER_STAGE_VERTEX_BIT;
        stages[0].module = first;
        stages[0].pName = "main";
        stages[1].sType = VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO;
        stages[1].stage = VK_SHADER_STAGE_FRAGMENT_BIT;
        stages[1].module = second;
        stages[1].pName = "main";

        VkPipelineVertexInputStateCreateInfo nothing_fetched = {0};
        nothing_fetched.sType = VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO;
        VkPipelineInputAssemblyStateCreateInfo shape = {0};
        shape.sType = VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO;
        shape.topology = VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST;
        VkPipelineViewportStateCreateInfo where = {0};
        where.sType = VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO;
        where.viewportCount = 1;
        where.scissorCount = 1;
        const VkDynamicState changes[] = {VK_DYNAMIC_STATE_SCISSOR, VK_DYNAMIC_STATE_VIEWPORT};
        VkPipelineDynamicStateCreateInfo said_per_draw = {0};
        said_per_draw.sType = VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO;
        said_per_draw.dynamicStateCount = 2;
        said_per_draw.pDynamicStates = changes;
        VkPipelineRasterizationStateCreateInfo filling = {0};
        filling.sType = VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO;
        filling.polygonMode = VK_POLYGON_MODE_FILL;
        filling.cullMode = VK_CULL_MODE_NONE;
        filling.lineWidth = 1.0f;
        VkPipelineMultisampleStateCreateInfo once = {0};
        once.sType = VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO;
        once.rasterizationSamples = VK_SAMPLE_COUNT_1_BIT;
        VkPipelineColorBlendAttachmentState keeping = {0};
        keeping.colorWriteMask = how_far ? 0u
                                         : VK_COLOR_COMPONENT_R_BIT | VK_COLOR_COMPONENT_G_BIT
                                               | VK_COLOR_COMPONENT_B_BIT
                                               | VK_COLOR_COMPONENT_A_BIT;
        VkPipelineColorBlendStateCreateInfo mixing = {0};
        mixing.sType = VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO;
        mixing.attachmentCount = 1;
        mixing.pAttachments = &keeping;
        VkPipelineDepthStencilStateCreateInfo how_near = {0};
        how_near.sType = VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO;
        how_near.depthTestEnable = how_far ? VK_TRUE : VK_FALSE;
        how_near.depthWriteEnable = how_far ? VK_TRUE : VK_FALSE;
        how_near.depthCompareOp = VK_COMPARE_OP_ALWAYS;
        how_near.maxDepthBounds = 1.0f;

        VkGraphicsPipelineCreateInfo asking = {0};
        asking.sType = VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO;
        asking.stageCount = 2;
        asking.pStages = stages;
        asking.pVertexInputState = &nothing_fetched;
        asking.pInputAssemblyState = &shape;
        asking.pViewportState = &where;
        asking.pRasterizationState = &filling;
        asking.pMultisampleState = &once;
        asking.pDepthStencilState = &how_near;
        asking.pColorBlendState = &mixing;
        asking.pDynamicState = &said_per_draw;
        asking.layout = layout;
        asking.renderPass = pass;
        if (vkCreateGraphicsPipelines(drawing.device, VK_NULL_HANDLE, 1, &asking, NULL, &made)
            != VK_SUCCESS) {
            made = VK_NULL_HANDLE;
        }
    }
    if (first != VK_NULL_HANDLE) {
        vkDestroyShaderModule(drawing.device, first, NULL);
    }
    if (second != VK_NULL_HANDLE) {
        vkDestroyShaderModule(drawing.device, second, NULL);
    }
    return made;
}

static int make_what_moving_round_takes(void) {
    if (rearranging.made || rearranging.refused) {
        return rearranging.made;
    }
    rearranging.refused = 1;
    for (int how_far = 0; how_far < 2; how_far++) {
        VkImage held = somewhere_to_move_samples_round(how_far);
        if (held == VK_NULL_HANDLE) {
            return 0;
        }
        VkImageViewCreateInfo seeing = {0};
        seeing.sType = VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO;
        seeing.image = held;
        seeing.viewType = VK_IMAGE_VIEW_TYPE_2D;
        seeing.format = how_far ? XENOLITH_HOW_FAR_FORMAT : XENOLITH_SURFACE_FORMAT;
        seeing.subresourceRange.aspectMask =
            how_far ? VK_IMAGE_ASPECT_DEPTH_BIT : VK_IMAGE_ASPECT_COLOR_BIT;
        seeing.subresourceRange.levelCount = 1;
        seeing.subresourceRange.layerCount = 1;
        if (vkCreateImageView(drawing.device, &seeing, NULL, &rearranging.seen[how_far])
            != VK_SUCCESS) {
            return 0;
        }
    }

    VkSamplerCreateInfo reading = {0};
    reading.sType = VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO;
    reading.magFilter = VK_FILTER_NEAREST;
    reading.minFilter = VK_FILTER_NEAREST;
    reading.addressModeU = VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE;
    reading.addressModeV = VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE;
    reading.addressModeW = VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE;
    if (vkCreateSampler(drawing.device, &reading, NULL, &rearranging.way) != VK_SUCCESS) {
        return 0;
    }

    VkDescriptorSetLayoutBinding one_picture = {0};
    one_picture.binding = 0;
    one_picture.descriptorType = VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER;
    one_picture.descriptorCount = 1;
    one_picture.stageFlags = VK_SHADER_STAGE_FRAGMENT_BIT;
    VkDescriptorSetLayoutCreateInfo shaped = {0};
    shaped.sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO;
    shaped.bindingCount = 1;
    shaped.pBindings = &one_picture;
    if (vkCreateDescriptorSetLayout(drawing.device, &shaped, NULL, &rearranging.shape)
        != VK_SUCCESS) {
        return 0;
    }

    VkDescriptorPoolSize enough = {VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, 2};
    VkDescriptorPoolCreateInfo pool = {0};
    pool.sType = VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO;
    pool.maxSets = 2;
    pool.poolSizeCount = 1;
    pool.pPoolSizes = &enough;
    if (vkCreateDescriptorPool(drawing.device, &pool, NULL, &rearranging.holding) != VK_SUCCESS) {
        return 0;
    }
    VkDescriptorSetLayout both[2] = {rearranging.shape, rearranging.shape};
    VkDescriptorSetAllocateInfo taking = {0};
    taking.sType = VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO;
    taking.descriptorPool = rearranging.holding;
    taking.descriptorSetCount = 2;
    taking.pSetLayouts = both;
    if (vkAllocateDescriptorSets(drawing.device, &taking, rearranging.said) != VK_SUCCESS) {
        return 0;
    }
    for (int how_far = 0; how_far < 2; how_far++) {
        VkDescriptorImageInfo picture = {0};
        picture.sampler = rearranging.way;
        picture.imageView = rearranging.seen[how_far];
        picture.imageLayout = VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL;
        VkWriteDescriptorSet saying = {0};
        saying.sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET;
        saying.dstSet = rearranging.said[how_far];
        saying.descriptorCount = 1;
        saying.descriptorType = VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER;
        saying.pImageInfo = &picture;
        vkUpdateDescriptorSets(drawing.device, 1, &saying, 0, NULL);
    }

    VkPushConstantRange which_ways = {VK_SHADER_STAGE_FRAGMENT_BIT, 0, 2u * sizeof(int32_t)};
    VkPipelineLayoutCreateInfo outside = {0};
    outside.sType = VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO;
    outside.setLayoutCount = 1;
    outside.pSetLayouts = &rearranging.shape;
    outside.pushConstantRangeCount = 1;
    outside.pPushConstantRanges = &which_ways;
    if (vkCreatePipelineLayout(drawing.device, &outside, NULL, &rearranging.layout)
        != VK_SUCCESS) {
        return 0;
    }
    for (int how_far = 0; how_far < 2; how_far++) {
        rearranging.pipeline[how_far] =
            how_far ? a_pipeline_of_our_own(moving_distances_round,
                                            sizeof moving_distances_round / 4u,
                                            rearranging.layout, through.pass, 1)
                    : a_pipeline_of_our_own(moving_colours_round,
                                            sizeof moving_colours_round / 4u,
                                            rearranging.layout, through.pass, 0);
        if (rearranging.pipeline[how_far] == VK_NULL_HANDLE) {
            return 0;
        }
    }
    rearranging.made = 1;
    rearranging.refused = 0;
    return 1;
}

/* Gets ready to lay the samples an image holds out the way the next draw wants
 * them, from the way the draws that made it had them.
 *
 * The console keeps the samples of a pixel drawn as more than one in a run of
 * four with its neighbour's: the first sample of each, then the second of
 * each. Here they are kept where they fall, a pixel's two side by side. Where
 * a title draws into the same memory both ways, with one sample to a pixel and
 * with more, what one way put in the second place the other reads in the third,
 * and a title relies on it: it lays a scene down a pixel at a time already
 * rearranged so that drawing over it with four samples to a pixel lines up.
 * Swapping the two each time the way of drawing changes is the same in both
 * directions.
 *
 * As much of the image as the place covers is copied aside here, while the
 * image is somewhere to copy into, and put back moved round by a draw once the
 * frame's pass has begun. Done with a copy for every column instead, it was
 * most of what a frame cost the device. */
static void ready_to_move_round(VkCommandBuffer on, VkImage image, int how_far, uint32_t was,
                                uint32_t now, uint32_t wide, uint32_t tall) {
    rearranging.waiting[how_far] = 0;
    if (was == now || wide == 0 || tall == 0) {
        return;
    }
    anything_recorded = 1;
    if (!make_what_moving_round_takes()) {
        return;
    }
    VkImageAspectFlags aspect = how_far ? VK_IMAGE_ASPECT_DEPTH_BIT : VK_IMAGE_ASPECT_COLOR_BIT;
    VkImageCopy all = {0};
    all.srcSubresource.aspectMask = aspect;
    all.srcSubresource.layerCount = 1;
    all.dstSubresource = all.srcSubresource;
    all.extent.width = wide < XENOLITH_GRID_ACROSS ? wide : XENOLITH_GRID_ACROSS;
    all.extent.height = tall < XENOLITH_GRID_DOWN ? tall : XENOLITH_GRID_DOWN;
    all.extent.depth = 1;

    VkImage held = moving_round[how_far].image;
    samples_become(on, image, aspect, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                   VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL);
    samples_become(on, held, aspect, VK_IMAGE_LAYOUT_UNDEFINED,
                   VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL);
    vkCmdCopyImage(on, image, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, held,
                   VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1, &all);
    samples_become(on, held, aspect, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                   VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL);
    samples_become(on, image, aspect, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                   VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL);

    rearranging.swapped[how_far][0] = (was & 3u) != (now & 3u);
    rearranging.swapped[how_far][1] = (was >> 2) != (now >> 2);
    rearranging.wide[how_far] = all.extent.width;
    rearranging.tall[how_far] = all.extent.height;
    rearranging.waiting[how_far] = 1;
}

/* Puts back moved round whatever was copied aside for the frame being begun,
 * once its pass has begun. */
static void move_round_what_is_waiting(VkCommandBuffer on) {
    for (int how_far = 0; how_far < 2; how_far++) {
        if (!rearranging.waiting[how_far]) {
            continue;
        }
        rearranging.waiting[how_far] = 0;
        VkViewport seen = {0};
        seen.width = (float)rearranging.wide[how_far];
        seen.height = (float)rearranging.tall[how_far];
        seen.maxDepth = 1.0f;
        VkRect2D kept = {0};
        kept.extent.width = rearranging.wide[how_far];
        kept.extent.height = rearranging.tall[how_far];
        vkCmdSetViewport(on, 0, 1, &seen);
        vkCmdSetScissor(on, 0, 1, &kept);
        vkCmdBindPipeline(on, VK_PIPELINE_BIND_POINT_GRAPHICS, rearranging.pipeline[how_far]);
        vkCmdBindDescriptorSets(on, VK_PIPELINE_BIND_POINT_GRAPHICS, rearranging.layout, 0, 1,
                                &rearranging.said[how_far], 0, NULL);
        vkCmdPushConstants(on, rearranging.layout, VK_SHADER_STAGE_FRAGMENT_BIT, 0,
                           sizeof rearranging.swapped[how_far], rearranging.swapped[how_far]);
        vkCmdDraw(on, 3, 1, 0, 0);
    }
}


static void finish_the_lot(uint32_t lot);

/* Gives the lot being recorded into its share of the room draws take. */
static void give_out_the_room_again(void) {
    room_given_out_again++;
    through.vertex_at = (VkDeviceSize)current_lot * XENOLITH_VERTEX_ROOM;
    through.constant_at = (VkDeviceSize)current_lot * XENOLITH_CONSTANT_ROOM;
    sayings_taken = 0;
}

/* Begins recording into the lot whose turn it is, once the device has
 * finished with what was recorded into it last. */
static int begin_the_batch(void) {
    if (batch_open) {
        return 1;
    }
    finish_the_lot(current_lot);
    batches_begun++;
    surface.commands = lots[current_lot].main;
    vkResetCommandBuffer(surface.commands, 0);
    VkCommandBufferBeginInfo starting = {0};
    starting.sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO;
    starting.flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT;
    if (vkBeginCommandBuffer(surface.commands, &starting) != VK_SUCCESS) {
        return 0;
    }
    give_out_the_room_again();
    batch_open = 1;
    return 1;
}

/* Where a picture being set up is handed over: the lot that runs just before
 * the one being recorded into. */
static VkCommandBuffer commands_to_set_up_with(void) {
    if (!begin_the_batch()) {
        return VK_NULL_HANDLE;
    }
    if (!lots[current_lot].before_open) {
        vkResetCommandBuffer(lots[current_lot].before, 0);
        VkCommandBufferBeginInfo starting = {0};
        starting.sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO;
        starting.flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT;
        if (vkBeginCommandBuffer(lots[current_lot].before, &starting) != VK_SUCCESS) {
            return VK_NULL_HANDLE;
        }
        lots[current_lot].before_open = 1;
    }
    anything_recorded = 1;
    return lots[current_lot].before;
}

static int xenolith_begin_a_frame_inside(void) {
    if (recording || !make_what_a_frame_goes_through() || !begin_the_batch()) {
        return 0;
    }

    /* Where a title has turned to another place, what the place it leaves holds
     * is put aside and what the place it takes up holds is brought back. */
    int came_back_to_it = keep_the_place_and_take_up_another();

    VkImageMemoryBarrier ready = {0};
    ready.sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER;
    /* Where the surface is now, said as it actually is. A device starts an
     * image in the layout it was made with, which is undefined, and saying it
     * is somewhere else is undefined behaviour: one driver here refuses the
     * whole command buffer for it and another discards what is there. After a
     * frame it is where the copy out left it, and coming back to a place that
     * was put back leaves it where putting it back left it. */
    if (!surface.read_out) {
        ready.oldLayout = VK_IMAGE_LAYOUT_UNDEFINED;
    } else if (surface_brought_back) {
        ready.oldLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
    } else {
        ready.oldLayout = VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
    }
    ready.newLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
    ready.srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
    ready.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
    ready.image = surface.image;
    ready.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
    ready.subresourceRange.levelCount = 1;
    ready.subresourceRange.layerCount = 1;
    ready.srcAccessMask = VK_ACCESS_TRANSFER_READ_BIT | VK_ACCESS_TRANSFER_WRITE_BIT;
    ready.dstAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;
    vkCmdPipelineBarrier(surface.commands, VK_PIPELINE_STAGE_TRANSFER_BIT,
                         VK_PIPELINE_STAGE_TRANSFER_BIT, 0, 0, NULL, 0, NULL, 1, &ready);

    /* A place is wiped where a title asked for it through its last copy, and
     * where it has not been drawn into before. Otherwise what it left in a
     * place is what it means to find when it comes back to it.
     *
     * A title that asks and is not obliged goes on drawing over what it drew
     * before, and what it meant to lay over a fresh start piles up instead
     * until every colour is as high as it goes and the whole comes out white. */
    if (!came_back_to_it || wipe_the_colours) {
        VkClearColorValue cleared_to = {0};
        cleared_to.float32[0] = (float)((colours_start_at >> 16) & 0xffu) / 255.0f;
        cleared_to.float32[1] = (float)((colours_start_at >> 8) & 0xffu) / 255.0f;
        cleared_to.float32[2] = (float)(colours_start_at & 0xffu) / 255.0f;
        cleared_to.float32[3] = wipe_the_colours
                                    ? (float)((colours_start_at >> 24) & 0xffu) / 255.0f
                                    : 1.0f;
        vkCmdClearColorImage(surface.commands, surface.image,
                             VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, &cleared_to, 1,
                             &ready.subresourceRange);
        anything_recorded = 1;
    }
    if (came_back_to_it && !wipe_the_colours) {
        ready_to_move_round(surface.commands, surface.image, 0, surface_laid, drawing_laid,
                            drawing_across, surface_down);
    }
    surface_laid = drawing_laid;
    wipe_the_colours = 0;

    VkImageMemoryBarrier to_draw_on = ready;
    to_draw_on.oldLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
    to_draw_on.newLayout = VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL;
    to_draw_on.srcAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;
    to_draw_on.dstAccessMask = VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT;
    vkCmdPipelineBarrier(surface.commands, VK_PIPELINE_STAGE_TRANSFER_BIT,
                         VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT, 0, 0, NULL, 0, NULL, 1,
                         &to_draw_on);

    /* And everywhere starts as far away as the view reaches, so that the first
     * thing drawn at a place is nearer than what was there.
     *
     * How far away things are is only made somewhere to copy into and back
     * where it is to be wiped or moved round. Every change of what an image is
     * for makes a device unpack the whole of it and pack it again, and making
     * it so for every frame begun, a few dozen times a frame, was most of what
     * a frame cost the device. Left as it is, it is only told that the frame
     * before has finished with it. */
    int how_far_moves = wipe_how_far
                        || (how_far_laid != drawing_laid && drawing_across != 0
                            && surface_down != 0);
    VkImageMemoryBarrier how_far_ready = ready;
    how_far_ready.image = through.how_far;
    how_far_ready.subresourceRange.aspectMask = VK_IMAGE_ASPECT_DEPTH_BIT;
    how_far_ready.srcAccessMask = VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT;
    if (!how_far_moves) {
        how_far_ready.oldLayout = VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL;
        how_far_ready.newLayout = VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL;
        how_far_ready.dstAccessMask = VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT
                                      | VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT;
        vkCmdPipelineBarrier(surface.commands, VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
                             VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT, 0, 0, NULL, 0, NULL, 1,
                             &how_far_ready);
    } else {
        how_far_ready.oldLayout = wipe_how_far ? VK_IMAGE_LAYOUT_UNDEFINED
                                               : VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL;
        vkCmdPipelineBarrier(surface.commands, VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
                             VK_PIPELINE_STAGE_TRANSFER_BIT, 0, 0, NULL, 0, NULL, 1,
                             &how_far_ready);

        if (wipe_how_far) {
            VkClearDepthStencilValue as_far_as_it_goes = {how_far_to_start_at, 0};
            vkCmdClearDepthStencilImage(surface.commands, through.how_far,
                                        VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, &as_far_as_it_goes,
                                        1, &how_far_ready.subresourceRange);
            anything_recorded = 1;
        } else {
            ready_to_move_round(surface.commands, through.how_far, 1, how_far_laid, drawing_laid,
                                drawing_across, surface_down);
        }

        VkImageMemoryBarrier how_far_to_draw_on = how_far_ready;
        how_far_to_draw_on.oldLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
        how_far_to_draw_on.newLayout = VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL;
        how_far_to_draw_on.srcAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;
        how_far_to_draw_on.dstAccessMask = VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT;
        vkCmdPipelineBarrier(surface.commands, VK_PIPELINE_STAGE_TRANSFER_BIT,
                             VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT, 0, 0, NULL, 0, NULL, 1,
                             &how_far_to_draw_on);
    }
    how_far_laid = drawing_laid;
    wipe_how_far = 0;

    VkRenderPassBeginInfo into = {0};
    into.sType = VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO;
    into.renderPass = through.pass;
    into.framebuffer = through.target;
    into.renderArea.extent.width = XENOLITH_GRID_ACROSS;
    into.renderArea.extent.height = XENOLITH_GRID_DOWN;
    vkCmdBeginRenderPass(surface.commands, &into, VK_SUBPASS_CONTENTS_INLINE);
    move_round_what_is_waiting(surface.commands);
    if (seen_given) {
        vkCmdSetViewport(surface.commands, 0, 1, &last_seen);
    }
    if (kept_given) {
        vkCmdSetScissor(surface.commands, 0, 1, &last_kept);
    }

    recording = 1;
    return 1;
}

/* Showing a frame records into the shared command pool, so it is kept to the
 * same lock that drawing a frame is. Recursive, so the end-of-frame path that
 * already holds it reaches here, and the path that shows a copied frame on its
 * own thread is serialised against a frame being drawn. */
/* Shows one picture in the window.
 *
 * Which picture is given rather than always being the surface. The surface is
 * what every thread draws into, so between a frame being finished and the
 * window being handed it, another thread has usually started the next one:
 * that is a window showing a frame half drawn, and it is what made the screen
 * go black between the frames it did show. */
static VkImage what_was_shown;

#ifdef XENOLITH_WITH_WINDOW

static void show_what_was_drawn_inside(VkImage what);

#endif

static void show_what_was_drawn(VkImage what) {
    what_was_shown = what;
#ifdef XENOLITH_WITH_WINDOW
    pthread_mutex_lock(&one_drawing);
    show_what_was_drawn_inside(what);
    pthread_mutex_unlock(&one_drawing);
#else
    (void)what;
#endif
}


/* The drawing, one thread at a time. Each of these records into the one buffer
 * a frame is built in, or hands that buffer over, so none of them may be
 * happening while another is. */
int xenolith_begin_a_frame(void) {
    pthread_mutex_lock(&one_drawing);
    int went = xenolith_begin_a_frame_inside();
    pthread_mutex_unlock(&one_drawing);
    return went;
}

int xenolith_end_a_frame(void) {
    pthread_mutex_lock(&one_drawing);
    int went = xenolith_end_a_frame_inside();
    pthread_mutex_unlock(&one_drawing);
    return went;
}

int xenolith_draw_now(uint32_t blending, uint32_t written, uint32_t how_far, uint32_t cut_off,
                      uint32_t cut_off_at, uint32_t vertex_sum, const uint32_t *vertex,
                      uint32_t vertex_length, uint32_t pixel_sum, const uint32_t *pixel,
                      uint32_t pixel_length, uint32_t vertices, uint32_t kind) {
    pthread_mutex_lock(&one_drawing);
    int drawn = xenolith_draw_now_inside(blending, written, how_far, cut_off, cut_off_at,
                                         vertex_sum, vertex, vertex_length, pixel_sum, pixel,
                                         pixel_length, vertices, kind);
    pthread_mutex_unlock(&one_drawing);
    return drawn;
}

void xenolith_clip_to(uint32_t left, uint32_t top, uint32_t right, uint32_t bottom) {
    pthread_mutex_lock(&one_drawing);
    xenolith_clip_to_inside(left, top, right, bottom);
    pthread_mutex_unlock(&one_drawing);
}

/* Where on what is drawn into a position between one and minus one lands.
 *
 * A console says this as what to multiply a position by and what to add, and a
 * host says it as a rectangle. The two are the same thing written differently:
 * multiplying by half the width and adding the middle is what putting the
 * rectangle there comes to, so the rectangle is worked back out of the pair.
 *
 * The height comes out negative, because a console counts upward from the
 * bottom and what is drawn into counts downward from the top, and that is
 * exactly what the title's own numbers say. A host takes a negative height and
 * means the same by it.
 */
static void xenolith_look_at_inside(const float *scale, const float *offset) {
    if (!recording) {
        return;
    }

    VkViewport seen = {0};
    seen.x = offset[0] - scale[0];
    seen.y = offset[1] - scale[1];
    seen.width = scale[0] * 2.0f;
    seen.height = scale[1] * 2.0f;
    seen.minDepth = offset[2];
    seen.maxDepth = offset[2] + scale[2];
    if (seen.minDepth > seen.maxDepth) {
        float swap = seen.minDepth;
        seen.minDepth = seen.maxDepth;
        seen.maxDepth = swap;
    }
    if (seen.minDepth < 0.0f) {
        seen.minDepth = 0.0f;
    }
    if (seen.maxDepth > 1.0f) {
        seen.maxDepth = 1.0f;
    }
    last_seen = seen;
    seen_given = 1;
    vkCmdSetViewport(surface.commands, 0, 1, &seen);
}

void xenolith_look_at(const float *scale, const float *offset) {
    const float whole_scale[4] = {(float)XENOLITH_ACROSS / 2.0f, (float)XENOLITH_DOWN / 2.0f, 1.0f,
                                  1.0f};
    const float whole_offset[4] = {(float)XENOLITH_ACROSS / 2.0f, (float)XENOLITH_DOWN / 2.0f, 0.0f,
                                   0.0f};

    pthread_mutex_lock(&one_drawing);
    xenolith_look_at_inside(scale != NULL ? scale : whole_scale,
                            offset != NULL ? offset : whole_offset);
    pthread_mutex_unlock(&one_drawing);
}
#ifdef XENOLITH_WITH_WINDOW

/* A window on the screen, and what is needed to put a frame into it.
 *
 * Opening one is not required for anything else here to work: a run with no
 * screen to open still draws, and still writes out what it drew when asked. So
 * failing to open one is reported once and then left alone.
 */
static struct {
#ifdef XENOLITH_WITH_X11
    Display *server;
    Window window;
#endif
    VkSurfaceKHR surface;
    VkSwapchainKHR chain;
    VkImage images[8];
    uint32_t count;
    VkSemaphore taken;
    VkSemaphore shown;
    VkCommandBuffer copying;
    /* Says the last copy into the window has run, so that its commands can be
     * recorded again. Waited for there rather than after every showing, which
     * waited for everything the device had been given, the frame being drawn
     * behind it included. */
    VkFence copied;
    /* What the window manager may say to this window. A window that never
     * answers these is a window the rest of the desktop has to decide about on
     * its own, and what it decides is that the program is not answering. */
#ifdef XENOLITH_WITH_X11
    Atom talking;
    Atom closing;
    Atom pinging;
#endif
    VkExtent2D across;
    VkPresentModeKHR showing;
    int the_desktops_own_way;
    int opened;
    int refused;
} screen;

#ifdef XENOLITH_WITH_WAYLAND
static void stop_talking_to_the_desktop(void);
#endif

/* Gives back the window, the chain of images shown in it and the display.
 *
 * A run that simply stops leaves all three for whatever draws this machine's
 * screen to notice and clear up, and a run started and stopped many times over
 * leaves that many. None of it belongs to this program once it has gone, so it
 * is handed back rather than dropped. */
static void give_the_window_back(void) {
    if (!screen.opened) {
        return;
    }
    screen.opened = 0;
    if (drawing.device != VK_NULL_HANDLE) {
        vkDeviceWaitIdle(drawing.device);
        if (screen.chain != VK_NULL_HANDLE) {
            vkDestroySwapchainKHR(drawing.device, screen.chain, NULL);
            screen.chain = VK_NULL_HANDLE;
        }
        if (screen.taken != VK_NULL_HANDLE) {
            vkDestroySemaphore(drawing.device, screen.taken, NULL);
            screen.taken = VK_NULL_HANDLE;
        }
        if (screen.shown != VK_NULL_HANDLE) {
            vkDestroySemaphore(drawing.device, screen.shown, NULL);
            screen.shown = VK_NULL_HANDLE;
        }
    }
    if (screen.surface != VK_NULL_HANDLE && drawing.instance != VK_NULL_HANDLE) {
        vkDestroySurfaceKHR(drawing.instance, screen.surface, NULL);
        screen.surface = VK_NULL_HANDLE;
    }
    #ifdef XENOLITH_WITH_X11
    if (screen.server != NULL) {
        if (screen.window != 0) {
            XDestroyWindow(screen.server, screen.window);
            screen.window = 0;
        }
        XCloseDisplay(screen.server);
        screen.server = NULL;
    }
#endif
#ifdef XENOLITH_WITH_WAYLAND
    if (screen.the_desktops_own_way) {
        stop_talking_to_the_desktop();
        screen.the_desktops_own_way = 0;
    }
#endif
}

#ifdef XENOLITH_WITH_WAYLAND

/* Talking to the desktop its own way.
 *
 * A desktop that speaks this has to start a whole second display server to be
 * spoken to the older way, which is a great deal of machinery to ask for on
 * behalf of one window, and it is started and stopped around every run. Saying
 * it directly asks for none of that.
 *
 * Only what one window needs is here: something to make a surface with,
 * something to give that surface a place and a title, and answering when the
 * desktop asks whether this program is still there.
 */
static struct {
    struct wl_display *desktop;
    struct wl_registry *offered;
    struct wl_compositor *making;
    struct xdg_wm_base *placing;
    struct wl_surface *surface;
    struct xdg_surface *placed;
    struct xdg_toplevel *window;
    int settled;
    int closed;
    uint32_t across;
    uint32_t down;
    struct wl_seat *seat;
    struct wl_keyboard *keys;
    /* What is held down now, as a controller, and where each thumbstick is.
     * Kept up to date as the desktop reports keys going down and coming up,
     * because the older way reads the whole keyboard at the moment it is asked
     * and this way is only told of changes. */
    uint16_t held;
    int16_t thumbs[4];
} speaking;

/* The desktop hands a key over as the hardware's own number, which is the same
 * whatever letters are printed on the keys, so what is mapped is where a key
 * sits rather than what it says. Enter and the arrows are where every keyboard
 * keeps them; the rest are where a QWERTY board keeps its letters, which is the
 * same choice the older way makes. These are the console's button bits. */
static const struct {
    uint32_t code;
    uint16_t button;
} desktop_keys_to_buttons[] = {
    {28u, 0x0010},  /* enter  -> start */
    {1u, 0x0020},   /* escape -> back */
    {103u, 0x0001}, /* up */
    {108u, 0x0002}, /* down */
    {105u, 0x0004}, /* left */
    {106u, 0x0008}, /* right */
    {44u, 0x1000},  /* z -> a */
    {45u, 0x2000},  /* x -> b */
    {46u, 0x4000},  /* c -> x */
    {47u, 0x8000},  /* v -> y */
    {16u, 0x0100},  /* q -> left shoulder */
    {18u, 0x0200},  /* e -> right shoulder */
};

/* A key standing in for a thumbstick, which of the two it moves, and which way.
 * The full throw is the same one the older way uses. */
static const struct {
    uint32_t code;
    unsigned int which;
    int16_t towards;
} desktop_keys_to_thumbs[] = {
    {17u, 1, 32767},   /* w -> left stick up */
    {31u, 1, -32767},  /* s -> left stick down */
    {30u, 0, -32767},  /* a -> left stick left */
    {32u, 0, 32767},   /* d -> left stick right */
};

/* The desktop asking whether this program is still answering. Saying so is the
 * whole of what it wants, and a program that does not is one the desktop has to
 * decide about on its own. */
static void answer_the_desktop(void *held, struct xdg_wm_base *placing, uint32_t said) {
    (void)held;
    xdg_wm_base_pong(placing, said);
}

static const struct xdg_wm_base_listener answering = {answer_the_desktop};

static void take_the_place_given(void *held, struct xdg_surface *placed, uint32_t said) {
    (void)held;
    xdg_surface_ack_configure(placed, said);
    speaking.settled = 1;
}

static const struct xdg_surface_listener placing_it = {take_the_place_given};

static void told_how_big(void *held, struct xdg_toplevel *window, int32_t across, int32_t down,
                         struct wl_array *how) {
    (void)held;
    (void)window;
    (void)how;
    if (across > 0 && down > 0) {
        speaking.across = (uint32_t)across;
        speaking.down = (uint32_t)down;
    }
}

static void told_to_close(void *held, struct xdg_toplevel *window) {
    (void)held;
    (void)window;
    speaking.closed = 1;
}

static const struct xdg_toplevel_listener being_a_window = {told_how_big, told_to_close, NULL,
                                                            NULL};

/* A key going down or coming up, given as the hardware's own number. What it
 * stands for is set or cleared where it is one of the keys mapped, and nothing
 * happens for the rest. */
static void a_key_moved(void *held, struct wl_keyboard *keyboard, uint32_t said, uint32_t when,
                        uint32_t code, uint32_t state) {
    (void)held;
    (void)keyboard;
    (void)said;
    (void)when;
    int down = state == 1u; /* WL_KEYBOARD_KEY_STATE_PRESSED */
    for (unsigned int at = 0; at < sizeof desktop_keys_to_buttons / sizeof *desktop_keys_to_buttons;
         at++) {
        if (desktop_keys_to_buttons[at].code == code) {
            if (down) {
                speaking.held |= desktop_keys_to_buttons[at].button;
            } else {
                speaking.held &= (uint16_t)~desktop_keys_to_buttons[at].button;
            }
        }
    }
    for (unsigned int at = 0; at < sizeof desktop_keys_to_thumbs / sizeof *desktop_keys_to_thumbs;
         at++) {
        if (desktop_keys_to_thumbs[at].code == code) {
            speaking.thumbs[desktop_keys_to_thumbs[at].which] =
                down ? desktop_keys_to_thumbs[at].towards : 0;
        }
    }
}

/* The keymap the desktop hands over, which says what letters the keys carry.
 * The positions are what is mapped here rather than the letters, so nothing is
 * read out of it, and the file it comes on is given straight back. */
static void a_keymap_was_given(void *held, struct wl_keyboard *keyboard, uint32_t format,
                               int32_t file, uint32_t size) {
    (void)held;
    (void)keyboard;
    (void)format;
    (void)size;
    if (file >= 0) {
        close(file);
    }
}

/* The window losing the keyboard, at which point nothing it held is held any
 * longer: a key let go while another window has the keyboard is never reported
 * here, so what was down would otherwise stay down for ever. */
static void the_keyboard_left(void *held, struct wl_keyboard *keyboard, uint32_t said,
                              struct wl_surface *of) {
    (void)held;
    (void)keyboard;
    (void)said;
    (void)of;
    speaking.held = 0;
    for (unsigned int at = 0; at < 4u; at++) {
        speaking.thumbs[at] = 0;
    }
}

static void the_keyboard_came(void *held, struct wl_keyboard *keyboard, uint32_t said,
                              struct wl_surface *of, struct wl_array *keys) {
    (void)held;
    (void)keyboard;
    (void)said;
    (void)of;
    (void)keys;
}

static void the_keys_changed(void *held, struct wl_keyboard *keyboard, uint32_t said,
                             uint32_t depressed, uint32_t latched, uint32_t locked,
                             uint32_t group) {
    (void)held;
    (void)keyboard;
    (void)said;
    (void)depressed;
    (void)latched;
    (void)locked;
    (void)group;
}

static void how_fast_it_repeats(void *held, struct wl_keyboard *keyboard, int32_t rate,
                                int32_t delay) {
    (void)held;
    (void)keyboard;
    (void)rate;
    (void)delay;
}

static const struct wl_keyboard_listener the_keyboard = {
    a_keymap_was_given, the_keyboard_came, the_keyboard_left,
    a_key_moved,        the_keys_changed,  how_fast_it_repeats,
};

/* What the seat can do, which says whether there is a keyboard to read. One
 * turning up is taken; one going away is given back, so a keyboard unplugged
 * mid run does not leave a dead handle behind. */
static void what_the_seat_can_do(void *held, struct wl_seat *seat, uint32_t can) {
    (void)held;
    int has_keyboard = (can & 2u) != 0; /* WL_SEAT_CAPABILITY_KEYBOARD */
    if (has_keyboard && speaking.keys == NULL) {
        speaking.keys = wl_seat_get_keyboard(seat);
        wl_keyboard_add_listener(speaking.keys, &the_keyboard, NULL);
        fprintf(stderr, "xenolith: reading the keyboard the desktop's own way; enter is start, "
                        "the arrows are the pad, z and x are a and b\n");
    } else if (!has_keyboard && speaking.keys != NULL) {
        wl_keyboard_destroy(speaking.keys);
        speaking.keys = NULL;
    }
}

static void the_seat_is_called(void *held, struct wl_seat *seat, const char *name) {
    (void)held;
    (void)seat;
    (void)name;
}

static const struct wl_seat_listener the_seat = {what_the_seat_can_do, the_seat_is_called};

static void what_the_desktop_offers(void *held, struct wl_registry *offered, uint32_t which,
                                    const char *what, uint32_t version) {
    (void)held;
    if (strcmp(what, wl_compositor_interface.name) == 0) {
        speaking.making = wl_registry_bind(offered, which, &wl_compositor_interface,
                                           version < 4u ? version : 4u);
    } else if (strcmp(what, xdg_wm_base_interface.name) == 0) {
        speaking.placing = wl_registry_bind(offered, which, &xdg_wm_base_interface, 1);
        xdg_wm_base_add_listener(speaking.placing, &answering, NULL);
    } else if (strcmp(what, wl_seat_interface.name) == 0) {
        speaking.seat =
            wl_registry_bind(offered, which, &wl_seat_interface, version < 5u ? version : 5u);
        wl_seat_add_listener(speaking.seat, &the_seat, NULL);
    }
}

static void what_the_desktop_took_back(void *held, struct wl_registry *offered, uint32_t which) {
    (void)held;
    (void)offered;
    (void)which;
}

static const struct wl_registry_listener listening = {what_the_desktop_offers,
                                                      what_the_desktop_took_back};

/* Gives back everything taken from the desktop, in the order it was taken. */
static void stop_talking_to_the_desktop(void) {
    if (speaking.window != NULL) {
        xdg_toplevel_destroy(speaking.window);
        speaking.window = NULL;
    }
    if (speaking.placed != NULL) {
        xdg_surface_destroy(speaking.placed);
        speaking.placed = NULL;
    }
    if (speaking.surface != NULL) {
        wl_surface_destroy(speaking.surface);
        speaking.surface = NULL;
    }
    if (speaking.keys != NULL) {
        wl_keyboard_destroy(speaking.keys);
        speaking.keys = NULL;
    }
    if (speaking.seat != NULL) {
        wl_seat_destroy(speaking.seat);
        speaking.seat = NULL;
    }
    if (speaking.placing != NULL) {
        xdg_wm_base_destroy(speaking.placing);
        speaking.placing = NULL;
    }
    if (speaking.making != NULL) {
        wl_compositor_destroy(speaking.making);
        speaking.making = NULL;
    }
    if (speaking.offered != NULL) {
        wl_registry_destroy(speaking.offered);
        speaking.offered = NULL;
    }
    if (speaking.desktop != NULL) {
        wl_display_disconnect(speaking.desktop);
        speaking.desktop = NULL;
    }
}

/* Makes a window the desktop's own way, or says nothing was made. */
static int talk_to_the_desktop(void) {
    speaking.across = XENOLITH_ACROSS;
    speaking.down = XENOLITH_DOWN;

    speaking.desktop = wl_display_connect(NULL);
    if (speaking.desktop == NULL) {
        return 0;
    }

    speaking.offered = wl_display_get_registry(speaking.desktop);
    wl_registry_add_listener(speaking.offered, &listening, NULL);
    wl_display_roundtrip(speaking.desktop);

    if (speaking.making == NULL || speaking.placing == NULL) {
        say_no_frame_will_be_shown("the desktop offers no way to make a window");
        stop_talking_to_the_desktop();
        return 0;
    }

    speaking.surface = wl_compositor_create_surface(speaking.making);
    speaking.placed = xdg_wm_base_get_xdg_surface(speaking.placing, speaking.surface);
    xdg_surface_add_listener(speaking.placed, &placing_it, NULL);
    speaking.window = xdg_surface_get_toplevel(speaking.placed);
    xdg_toplevel_add_listener(speaking.window, &being_a_window, NULL);
    xdg_toplevel_set_title(speaking.window, "xenolith");
    xdg_toplevel_set_app_id(speaking.window, "xenolith");
    wl_surface_commit(speaking.surface);

    /* The desktop says where the window goes before anything may be shown in
     * it, so this waits for that to be said. */
    while (!speaking.settled) {
        if (wl_display_dispatch(speaking.desktop) < 0) {
            stop_talking_to_the_desktop();
            return 0;
        }
    }
    return 1;
}

#endif

/* Makes the chain of images the window is shown through, at whatever size the
 * window is now.
 *
 * A window is whatever size the desktop decided to give it, which on a desktop
 * that lays windows out for you is not the size asked for and changes whenever
 * the layout does. A chain made once at the size a console draws leaves most of
 * the window untouched and every frame after a change refused, which shows as a
 * picture that does not fill and that comes and goes.
 */
static int make_a_chain_of_images(void) {
    VkSurfaceCapabilitiesKHR can;

    if (vkGetPhysicalDeviceSurfaceCapabilitiesKHR(drawing.adapter, screen.surface, &can)
        != VK_SUCCESS) {
        return 0;
    }

    VkExtent2D wants = can.currentExtent;
    if (wants.width == 0xffffffffu || wants.height == 0xffffffffu) {
        /* The desktop is letting this window choose, so it asks for what a
         * console draws. */
        wants.width = XENOLITH_ACROSS;
        wants.height = XENOLITH_DOWN;
    }
    if (wants.width < can.minImageExtent.width) {
        wants.width = can.minImageExtent.width;
    }
    if (wants.height < can.minImageExtent.height) {
        wants.height = can.minImageExtent.height;
    }
    if (wants.width > can.maxImageExtent.width) {
        wants.width = can.maxImageExtent.width;
    }
    if (wants.height > can.maxImageExtent.height) {
        wants.height = can.maxImageExtent.height;
    }
    if (wants.width == 0 || wants.height == 0) {
        /* Put away rather than shown. There is nothing to make a chain against
         * and nothing to show in it. */
        return 0;
    }

    /* Three rather than two, so that one can be being shown, one waiting to be,
     * and one drawn into, instead of a title waiting for one of two to come
     * free. */
    uint32_t at_least = can.minImageCount < 3u ? 3u : can.minImageCount;
    if (can.maxImageCount != 0 && at_least > can.maxImageCount) {
        at_least = can.maxImageCount;
    }

    VkSwapchainKHR was = screen.chain;

    VkSwapchainCreateInfoKHR making = {0};
    making.sType = VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR;
    making.surface = screen.surface;
    making.minImageCount = at_least;
    making.imageFormat = VK_FORMAT_B8G8R8A8_UNORM;
    making.imageColorSpace = VK_COLOR_SPACE_SRGB_NONLINEAR_KHR;
    making.imageExtent = wants;
    making.imageArrayLayers = 1;
    making.imageUsage = VK_IMAGE_USAGE_TRANSFER_DST_BIT;
    making.imageSharingMode = VK_SHARING_MODE_EXCLUSIVE;
    making.preTransform = can.currentTransform;
    making.compositeAlpha = VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR;
    making.presentMode = screen.showing;
    making.clipped = VK_TRUE;
    making.oldSwapchain = was;

    VkSwapchainKHR made = VK_NULL_HANDLE;
    if (vkCreateSwapchainKHR(drawing.device, &making, NULL, &made) != VK_SUCCESS) {
        say_no_frame_will_be_shown("no chain of images was made to draw into");
        return 0;
    }
    if (was != VK_NULL_HANDLE) {
        vkDeviceWaitIdle(drawing.device);
        vkDestroySwapchainKHR(drawing.device, was, NULL);
        if (screen.across.width != wants.width || screen.across.height != wants.height) {
            fprintf(stderr, "xenolith: the window is now %u by %u\n", wants.width, wants.height);
        }
    }
    screen.chain = made;
    screen.across = wants;

    screen.count = 8;
    if (vkGetSwapchainImagesKHR(drawing.device, screen.chain, &screen.count, screen.images)
        != VK_SUCCESS) {
        return 0;
    }
    return 1;
}

static int finish_opening_a_window(void);

/* Opens a window and the chain of images shown in it. */
static int open_a_window(void) {
    if (screen.opened || screen.refused) {
        return screen.opened;
    }

    /* A run that is being measured rather than watched opens no window.
     *
     * Showing a frame means asking whatever draws this machine's screen to take
     * one, and a run that starts and stops over and over asks it to make and
     * throw away the means of doing that just as often. That is a great deal of
     * churn to put through something the rest of the machine is relying on, for
     * a run nobody is looking at. */
    if (getenv("XENOLITH_NO_WINDOW") != NULL) {
        screen.refused = 1;
        say_no_frame_will_be_shown("this run was asked not to show one");
        return 0;
    }

    if (!make_somewhere_to_draw()) {
        screen.refused = 1;
        return 0;
    }

#ifdef XENOLITH_WITH_WAYLAND
    /* The desktop's own way first, where the desktop speaks it, so that no
     * second display server is started underneath on its behalf. */
    if (getenv("WAYLAND_DISPLAY") != NULL && talk_to_the_desktop()) {
        VkWaylandSurfaceCreateInfoKHR saying = {0};
        saying.sType = VK_STRUCTURE_TYPE_WAYLAND_SURFACE_CREATE_INFO_KHR;
        saying.display = speaking.desktop;
        saying.surface = speaking.surface;

        if (vkCreateWaylandSurfaceKHR(drawing.instance, &saying, NULL, &screen.surface)
                != VK_SUCCESS
            || screen.surface == VK_NULL_HANDLE) {
            screen.refused = 1;
            screen.surface = VK_NULL_HANDLE;
            stop_talking_to_the_desktop();
            say_no_frame_will_be_shown("the window the desktop made took no drawing");
            return 0;
        }

        /* A desktop drawn by another device than the one chosen here makes a
         * window that device will not draw into. The older way still can,
         * because the display server underneath copies each frame across, so
         * that is tried before giving up. */
        VkBool32 will = VK_FALSE;
        vkGetPhysicalDeviceSurfaceSupportKHR(drawing.adapter, drawing.family, screen.surface,
                                             &will);
        if (will) {
            screen.the_desktops_own_way = 1;
            return finish_opening_a_window();
        }
#ifdef XENOLITH_WITH_X11
        vkDestroySurfaceKHR(drawing.instance, screen.surface, NULL);
        screen.surface = VK_NULL_HANDLE;
        stop_talking_to_the_desktop();
#else
        screen.the_desktops_own_way = 1;
        return finish_opening_a_window();
#endif
    }
#endif

#ifdef XENOLITH_WITH_X11
    screen.server = XOpenDisplay(NULL);
    if (screen.server == NULL) {
        const char *asked = getenv("DISPLAY");
        screen.refused = 1;
        /* Naming the one that was tried, because the usual reason for this is a
         * DISPLAY left pointing at a display that is no longer there, and a
         * refusal that does not say which one sends whoever reads it looking at
         * the drawing instead. */
        fprintf(stderr,
                "xenolith: no frame will be shown, because the display %s did not answer; the "
                "run carries on with nothing on screen\n",
                asked != NULL && *asked != '\0' ? asked : "named by nothing at all");
        return 0;
    }

    int which = DefaultScreen(screen.server);
    screen.window = XCreateSimpleWindow(screen.server, RootWindow(screen.server, which), 0, 0,
                                        XENOLITH_ACROSS, XENOLITH_DOWN, 0,
                                        BlackPixel(screen.server, which),
                                        BlackPixel(screen.server, which));
    XStoreName(screen.server, screen.window, "xenolith");

    /* Saying what this window answers to, so that the rest of the desktop can
     * ask it to close rather than killing it, and can ask whether it is still
     * there rather than deciding on its own that it is not. */
    screen.talking = XInternAtom(screen.server, "WM_PROTOCOLS", False);
    screen.closing = XInternAtom(screen.server, "WM_DELETE_WINDOW", False);
    screen.pinging = XInternAtom(screen.server, "_NET_WM_PING", False);
    Atom answers[2] = {screen.closing, screen.pinging};
    XSetWMProtocols(screen.server, screen.window, answers, 2);

    /* Which program this is and which process, both of which whatever draws the
     * screen uses to group windows and to know who to ask. */
    XClassHint kind;
    kind.res_name = (char *)"xenolith";
    kind.res_class = (char *)"Xenolith";
    XSetClassHint(screen.server, screen.window, &kind);
    Atom whose = XInternAtom(screen.server, "_NET_WM_PID", False);
    long who = (long)getpid();
    XChangeProperty(screen.server, screen.window, whose, XA_CARDINAL, 32, PropModeReplace,
                    (const unsigned char *)&who, 1);

    XMapWindow(screen.server, screen.window);
    /* Being told when the desktop has sized the window, which is how a window
     * comes to be another shape than the one it was made with. Nothing here
     * asked for that before, so a desktop's resize arrived and was never
     * heard, and the picture went on at the size it was made at. */
    XSelectInput(screen.server, screen.window, StructureNotifyMask);
    XFlush(screen.server);
    atexit(give_the_window_back);

    VkXlibSurfaceCreateInfoKHR asking = {0};
    asking.sType = VK_STRUCTURE_TYPE_XLIB_SURFACE_CREATE_INFO_KHR;
    asking.dpy = screen.server;
    asking.window = screen.window;

    if (vkCreateXlibSurfaceKHR(drawing.instance, &asking, NULL, &screen.surface) != VK_SUCCESS
        || screen.surface == VK_NULL_HANDLE) {
        screen.refused = 1;
        screen.surface = VK_NULL_HANDLE;
        say_no_frame_will_be_shown("the window the desktop made took no drawing");
        return 0;
    }

    return finish_opening_a_window();
#else

    (void)0;
    say_no_frame_will_be_shown("no way of asking the desktop for a window was built into this");
    screen.refused = 1;
    return 0;
#endif
}

/* Everything that is the same whichever way the window was made: whether the
 * device will show anything in it, how frames are handed over, the chain of
 * images and the means of copying into them. */
static int finish_opening_a_window(void) {
    VkBool32 will = VK_FALSE;
    vkGetPhysicalDeviceSurfaceSupportKHR(drawing.adapter, drawing.family, screen.surface, &will);
    if (!will) {
        screen.refused = 1;
        say_no_frame_will_be_shown("the device will not draw into the window it was given");
        return 0;
    }

    VkSurfaceCapabilitiesKHR can;
    if (vkGetPhysicalDeviceSurfaceCapabilitiesKHR(drawing.adapter, screen.surface, &can)
        != VK_SUCCESS) {
        screen.refused = 1;
        return 0;
    }

    /* Whichever way of showing a frame does not hold the title up.
     *
     * A console hands a finished frame to a display and carries straight on. A
     * host window need not take one at the rate the title makes them, and
     * waiting for it to is not a console's pacing being kept: it is a title
     * being made to run at the speed of somebody else's screen. Measured here,
     * waiting cost more than half a second for every frame, against a title
     * that means to make thirty of them a second.
     *
     * So a way that drops a frame rather than holding one back is asked for
     * first, and waiting for the display is only what is left when a device
     * offers nothing else. What is lost is frames nobody would have seen.
     *
     * Showing each frame the moment it is handed over comes second. It can
     * tear on a screen drawn to directly, but a window a desktop puts together
     * is not drawn to directly, and the title already keeps the console's own
     * pace. A device showing through another desktop's windows may offer only
     * that and waiting, and waiting there stood the title still for as long as
     * its window was hidden or out of focus, while its sound played on. */
    VkPresentModeKHR without_waiting = VK_PRESENT_MODE_FIFO_KHR;
    uint32_t how_many_ways = 0;
    vkGetPhysicalDeviceSurfacePresentModesKHR(drawing.adapter, screen.surface, &how_many_ways, NULL);
    if (how_many_ways > 0) {
        VkPresentModeKHR ways[8];
        if (how_many_ways > 8u) {
            how_many_ways = 8u;
        }
        if (vkGetPhysicalDeviceSurfacePresentModesKHR(drawing.adapter, screen.surface,
                                                      &how_many_ways, ways)
            == VK_SUCCESS) {
            for (uint32_t at = 0; at < how_many_ways; at++) {
                if (ways[at] == VK_PRESENT_MODE_MAILBOX_KHR) {
                    without_waiting = VK_PRESENT_MODE_MAILBOX_KHR;
                    break;
                }
                if (ways[at] == VK_PRESENT_MODE_IMMEDIATE_KHR) {
                    without_waiting = VK_PRESENT_MODE_IMMEDIATE_KHR;
                }
            }
        }
    }

    screen.showing = without_waiting;
    if (!make_a_chain_of_images()) {
        screen.refused = 1;
        return 0;
    }

    VkSemaphoreCreateInfo saying = {0};
    saying.sType = VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO;
    if (vkCreateSemaphore(drawing.device, &saying, NULL, &screen.taken) != VK_SUCCESS
        || vkCreateSemaphore(drawing.device, &saying, NULL, &screen.shown) != VK_SUCCESS) {
        screen.refused = 1;
        return 0;
    }

    VkCommandBufferAllocateInfo taking = {0};
    taking.sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO;
    taking.commandPool = surface.pool;
    taking.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY;
    taking.commandBufferCount = 1;
    VkFenceCreateInfo telling = {0};
    telling.sType = VK_STRUCTURE_TYPE_FENCE_CREATE_INFO;
    telling.flags = VK_FENCE_CREATE_SIGNALED_BIT;
    if (vkAllocateCommandBuffers(drawing.device, &taking, &screen.copying) != VK_SUCCESS
        || vkCreateFence(drawing.device, &telling, NULL, &screen.copied) != VK_SUCCESS) {
        screen.refused = 1;
        return 0;
    }

    screen.opened = 1;
    fprintf(stderr, "xenolith: showing what is drawn in a window\n");
    return 1;
}

/* How a frame is scaled to the window, asked of the run once.
 *
 * Blending is what a window of any other shape wants: every part of a frame
 * grows into several of the window's, and hardening each one into a block
 * turns every smooth edge into stairs. Naming the other way by setting
 * XENOLITH_SCALING to nearest is there for the runs that want the parts
 * kept whole, which is the way this scaled before. */
static VkFilter the_way_a_frame_is_scaled(void) {
    static int chosen = -1;
    if (chosen < 0) {
        const char *how = getenv("XENOLITH_SCALING");
        chosen = how != NULL && strcmp(how, "nearest") == 0;
    }
    return chosen ? VK_FILTER_NEAREST : VK_FILTER_LINEAR;
}

/* Puts what was drawn into the window. */
static void show_what_was_drawn_inside(VkImage what) {
    if (!open_a_window()) {
        return;
    }

#ifdef XENOLITH_WITH_WAYLAND
    if (screen.the_desktops_own_way) {
        /* Answering whatever the desktop has said since the last frame, which
         * includes what keys were pressed and asking whether this program is
         * still there.
         *
         * The socket has to be read before there is anything to answer, and
         * reading it must not wait: a frame goes out sixty times a second
         * whether a key was pressed or not, so the read is gated on the socket
         * having something waiting rather than on blocking until it does.
         * Without the read the keys would sit in the socket unseen, which is
         * how a window that showed a frame could still feel dead to the
         * keyboard. */
        /* Read without waiting, and without treading on the graphics driver,
         * which is holding the same connection open for its own frames.
         *
         * Announcing the intent to read first, reading only when the socket
         * has something, and handing what is read to every queue on the
         * connection is what keeps the two apart. Reading the plain way instead
         * waits when the default queue is empty, which it usually is because
         * the driver has already taken what arrived, so a frame would then go
         * out only when the next thing did, which a key press is: that is a
         * window that draws one frame per key pressed and is otherwise still.
         */
        while (wl_display_prepare_read(speaking.desktop) != 0) {
            wl_display_dispatch_pending(speaking.desktop);
        }
        wl_display_flush(speaking.desktop);
        struct pollfd waiting = {wl_display_get_fd(speaking.desktop), POLLIN, 0};
        if (poll(&waiting, 1, 0) > 0 && (waiting.revents & POLLIN) != 0) {
            wl_display_read_events(speaking.desktop);
        } else {
            wl_display_cancel_read(speaking.desktop);
        }
        wl_display_dispatch_pending(speaking.desktop);
        if (speaking.closed) {
            give_the_window_back();
            screen.refused = 1;
            return;
        }
    } else
#endif
#ifdef XENOLITH_WITH_X11
    /* Answering what the window manager says, rather than throwing it away.
     *
     * A ping asks whether this window is still there. One that goes unanswered
     * is taken for a program that has stopped answering, and the rest of the
     * desktop then has to decide what to do about it. The answer is the same
     * message sent back to the root window, which is all it is waiting for.
     *
     * A request to close is honoured by giving the window back and drawing on
     * without it, which is what a run nobody is watching does anyway. */
    while (screen.server != NULL && XPending(screen.server) > 0) {
        XEvent said;
        XNextEvent(screen.server, &said);
        if (said.type == ConfigureNotify) {
            /* The desktop has sized the window, so the window is made that
             * size. On a console's windowing system the size arrives done; on
             * X the window is told what it should be and makes itself so,
             * which is what reaches the drawing here as a new size to fill.
             * One that is already the size asked for is the window hearing
             * about its own change, and is left alone. */
            if ((said.xconfigure.width != (int)screen.across.width ||
                 said.xconfigure.height != (int)screen.across.height) &&
                screen.chain != VK_NULL_HANDLE) {
                XResizeWindow(screen.server, screen.window, (unsigned)said.xconfigure.width,
                              (unsigned)said.xconfigure.height);
                XFlush(screen.server);
            }
            continue;
        }
        if (said.type != ClientMessage || said.xclient.message_type != screen.talking) {
            continue;
        }
        if ((Atom)said.xclient.data.l[0] == screen.pinging) {
            said.xclient.window = RootWindow(screen.server, DefaultScreen(screen.server));
            XSendEvent(screen.server, said.xclient.window, False,
                       SubstructureNotifyMask | SubstructureRedirectMask, &said);
        } else if ((Atom)said.xclient.data.l[0] == screen.closing) {
            give_the_window_back();
            screen.refused = 1;
            return;
        }
    }
#else
        /* The desktop's own way is the only one built into this, so the other
         * half of the choice has nothing to do. */
        (void)0;
#endif

    /* Not waited for. A desktop hands an image back when it is ready for
     * another, and one whose window is hidden or out of focus may not hand one
     * back for a long while. Waiting held the title's frame in hand all that
     * time: the picture stood still while the sound, which keeps its own time,
     * played on, and the two never met again. A frame no image is free for is
     * one nobody would have seen. */
    uint32_t which = 0;
    VkResult got = vkAcquireNextImageKHR(drawing.device, screen.chain, 0, screen.taken,
                                         VK_NULL_HANDLE, &which);
    if (got == VK_ERROR_OUT_OF_DATE_KHR) {
        /* The window is no longer the size the chain was made for. Make it
         * again and show the next frame instead of this one; skipping without
         * making it again is what leaves a window showing whatever it last
         * held, on and off, for as long as the run lasts. */
        if (!make_a_chain_of_images()) {
            screen.refused = 1;
            give_the_window_back();
        }
        return;
    }
    if (got != VK_SUCCESS && got != VK_SUBOPTIMAL_KHR) {
        return;
    }

    vkWaitForFences(drawing.device, 1, &screen.copied, VK_TRUE, UINT64_MAX);
    vkResetFences(drawing.device, 1, &screen.copied);
    VkCommandBufferBeginInfo starting = {0};
    starting.sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO;
    starting.flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT;
    vkBeginCommandBuffer(screen.copying, &starting);

    VkImageMemoryBarrier ready = {0};
    ready.sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER;
    ready.oldLayout = VK_IMAGE_LAYOUT_UNDEFINED;
    ready.newLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
    ready.srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
    ready.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
    ready.image = screen.images[which];
    ready.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
    ready.subresourceRange.levelCount = 1;
    ready.subresourceRange.layerCount = 1;
    ready.dstAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;
    vkCmdPipelineBarrier(screen.copying, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                         VK_PIPELINE_STAGE_TRANSFER_BIT, 0, 0, NULL, 0, NULL, 1, &ready);

    /* The bars a picture that keeps its shape leaves are this window's colour,
     * and the window's colour is black. Cleared rather than left, because the
     * picture arriving as it is turned round leaves the rest of the window
     * holding whatever was there, which is nobody's choice. */
    VkClearColorValue black;
    black.float32[0] = 0.0f;
    black.float32[1] = 0.0f;
    black.float32[2] = 0.0f;
    black.float32[3] = 1.0f;
    VkImageSubresourceRange every_part = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1};
    vkCmdClearColorImage(screen.copying, screen.images[which],
                         VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, &black, 1, &every_part);

    /* The picture keeps its shape.
     *
     * A window is rarely the shape of what a console drew, and stretching
     * what was drawn to fill it says the picture was the window's to reshape.
     * It fills as much of the window as its shape allows and is letterboxed
     * in the other direction, centred, the way a display that was given a
     * picture of another shape would show it. */
    VkImageBlit over = {0};
    over.srcSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
    over.srcSubresource.layerCount = 1;
    over.srcOffsets[1].x = (int32_t)(drawn_across != 0 ? drawn_across : XENOLITH_ACROSS);
    over.srcOffsets[1].y = (int32_t)(drawn_down != 0 ? drawn_down : XENOLITH_DOWN);
    over.srcOffsets[1].z = 1;
    over.dstSubresource = over.srcSubresource;
    uint32_t across = screen.across.width;
    uint32_t down = screen.across.height;
    uint32_t drawn_wide = (uint32_t)over.srcOffsets[1].x;
    uint32_t drawn_tall = (uint32_t)over.srcOffsets[1].y;
    if ((uint64_t)drawn_wide * down <= (uint64_t)across * drawn_tall) {
        uint32_t shown = (uint32_t)((uint64_t)drawn_wide * down / drawn_tall);
        over.dstOffsets[0].x = (int32_t)((across - shown) / 2u);
        over.dstOffsets[1].x = over.dstOffsets[0].x + (int32_t)shown;
        over.dstOffsets[1].y = (int32_t)down;
    } else {
        uint32_t shown = (uint32_t)((uint64_t)drawn_tall * across / drawn_wide);
        over.dstOffsets[0].y = (int32_t)((down - shown) / 2u);
        over.dstOffsets[1].y = over.dstOffsets[0].y + (int32_t)shown;
        over.dstOffsets[1].x = (int32_t)across;
    }
    vkCmdBlitImage(screen.copying, what, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                   screen.images[which], VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1, &over,
                   the_way_a_frame_is_scaled());

    VkImageMemoryBarrier done = ready;
    done.oldLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
    done.newLayout = VK_IMAGE_LAYOUT_PRESENT_SRC_KHR;
    done.srcAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;
    done.dstAccessMask = 0;
    vkCmdPipelineBarrier(screen.copying, VK_PIPELINE_STAGE_TRANSFER_BIT,
                         VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT, 0, 0, NULL, 0, NULL, 1, &done);

    vkEndCommandBuffer(screen.copying);

    VkPipelineStageFlags after = VK_PIPELINE_STAGE_TRANSFER_BIT;
    VkSubmitInfo handing = {0};
    handing.sType = VK_STRUCTURE_TYPE_SUBMIT_INFO;
    handing.waitSemaphoreCount = 1;
    handing.pWaitSemaphores = &screen.taken;
    handing.pWaitDstStageMask = &after;
    handing.commandBufferCount = 1;
    handing.pCommandBuffers = &screen.copying;
    handing.signalSemaphoreCount = 1;
    handing.pSignalSemaphores = &screen.shown;

    pthread_mutex_lock(&one_queue);
    if (vkQueueSubmit(drawing.queue, 1, &handing, screen.copied) != VK_SUCCESS) {
        pthread_mutex_unlock(&one_queue);
        return;
    }

    VkPresentInfoKHR showing = {0};
    showing.sType = VK_STRUCTURE_TYPE_PRESENT_INFO_KHR;
    showing.waitSemaphoreCount = 1;
    showing.pWaitSemaphores = &screen.shown;
    showing.swapchainCount = 1;
    showing.pSwapchains = &screen.chain;
    showing.pImageIndices = &which;
    VkResult shown = vkQueuePresentKHR(drawing.queue, &showing);
    pthread_mutex_unlock(&one_queue);
    if (shown == VK_ERROR_OUT_OF_DATE_KHR || shown == VK_SUBOPTIMAL_KHR) {
        if (!make_a_chain_of_images()) {
            screen.refused = 1;
            give_the_window_back();
        }
    }
}



#endif


#ifdef XENOLITH_WITH_WINDOW

/* What is held down on the keyboard, as a controller.
 *
 * Which key stands for which button is a choice rather than something read out
 * of anything: a console has a controller and a host has a keyboard, and nothing
 * says how one maps onto the other. It is written here so that it is one place
 * and so that it is plainly a choice.
 */
#ifdef XENOLITH_WITH_X11
static const struct {
    KeySym key;
    uint16_t button;
} keys_to_buttons[] = {
    {XK_Return, 0x0010},    /* start */
    {XK_Escape, 0x0020},    /* back */
    {XK_Up, 0x0001},        {XK_Down, 0x0002},
    {XK_Left, 0x0004},      {XK_Right, 0x0008},
    {XK_z, 0x1000},         /* a */
    {XK_x, 0x2000},         /* b */
    {XK_c, 0x4000},         /* x */
    {XK_v, 0x8000},         /* y */
    {XK_q, 0x0100},         /* left shoulder */
    {XK_e, 0x0200},         /* right shoulder */
};

/* How far a thumbstick goes when a key stands in for it. */
#define XENOLITH_ALL_THE_WAY 32767

static const struct {
    KeySym key;
    unsigned int which;
    int16_t towards;
} keys_to_thumbs[] = {
    {XK_w, 1, XENOLITH_ALL_THE_WAY},  {XK_s, 1, -XENOLITH_ALL_THE_WAY},
    {XK_a, 0, -XENOLITH_ALL_THE_WAY}, {XK_d, 0, XENOLITH_ALL_THE_WAY},
};

/* Whether a key is held down now. */
static int held(const char *keyboard, KeySym key) {
    KeyCode code = XKeysymToKeycode(screen.server, key);
    if (code == 0) {
        return 0;
    }
    return (keyboard[code / 8] & (1 << (code % 8))) != 0;
}
#endif /* XENOLITH_WITH_X11 */

/* What a run with no keyboard is told is held.
 *
 * A run that is not shown in a window has nothing to read keys from, and a
 * title that only ever sees nothing held cannot be taken past the screen that
 * waits for a button. So a run can say what is held, as the number a title
 * would be given, and every way of reading a keyboard here gives that first.
 *
 * A number on its own is held for the whole run. A screen that waits for a
 * button to be pressed rather than held needs it to go down while the screen
 * is up, so a number can also be given a span of seconds from the start of the
 * run, as `0x10@40-41`, and several of them can be given apart from each other
 * by commas. Read once, and each span measured from the first time anything
 * asks.
 *
 * A span whose start ends in `f`, as `0x1000@9000f-9010f`, counts frames shown
 * instead, the same numbers a run writing its frames out names them by. How
 * fast frames come varies from one run to the next while what a title shows on
 * each does not, so a button pressed on a frame lands on the screen that frame
 * showed. */
#define XENOLITH_HOLDS 64u

static int what_a_run_says_is_held(uint16_t *buttons) {
    static int settled = -1;
    static unsigned count;
    static struct {
        uint16_t buttons;
        double from;
        double until;
        int in_frames;
    } holds[XENOLITH_HOLDS];
    static struct timespec began;

    if (settled < 0) {
        const char *asked = getenv("XENOLITH_HOLD");
        clock_gettime(CLOCK_MONOTONIC, &began);
        while (asked != NULL && *asked != '\0' && count < XENOLITH_HOLDS) {
            char *after;
            holds[count].buttons = (uint16_t)strtoul(asked, &after, 0);
            holds[count].from = 0.0;
            holds[count].until = 1e300;
            holds[count].in_frames = 0;
            if (*after == '@') {
                holds[count].from = strtod(after + 1, &after);
                if (*after == 'f') {
                    holds[count].in_frames = 1;
                    after++;
                }
                if (*after == '-') {
                    holds[count].until = strtod(after + 1, &after);
                    if (*after == 'f') {
                        after++;
                    }
                }
            }
            count++;
            asked = *after == ',' ? after + 1 : "";
        }
        settled = 1;
    }

    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    double since = (double)(now.tv_sec - began.tv_sec) + (double)(now.tv_nsec - began.tv_nsec) / 1e9;
    uint16_t told = 0;
    double shown = (double)__atomic_load_n(&frames_shown, __ATOMIC_RELAXED);
    for (unsigned at = 0; at < count; at++) {
        double when = holds[at].in_frames ? shown : since;
        if (when >= holds[at].from && when < holds[at].until) {
            told |= holds[at].buttons;
        }
    }
    *buttons |= told;
    return count != 0;
}

int xenolith_what_is_held(uint16_t *buttons, int16_t *thumbs) {
    if (buttons == NULL || thumbs == NULL) {
        return 0;
    }
    *buttons = 0;
    for (unsigned int at = 0; at < 4u; at++) {
        thumbs[at] = 0;
    }
    if (what_a_run_says_is_held(buttons)) {
        return 1;
    }

#ifdef XENOLITH_WITH_WAYLAND
    /* A window made the desktop's own way is read the desktop's own way: the
     * keys it reported going down and coming up, kept as they came. */
    if (screen.the_desktops_own_way) {
        *buttons = speaking.held;
        for (unsigned int at = 0; at < 4u; at++) {
            thumbs[at] = speaking.thumbs[at];
        }
        return 1;
    }
#endif

#ifdef XENOLITH_WITH_X11
    if (!screen.opened || screen.server == NULL) {
        /* No window, or one made the desktop's own way, which is not something
         * the keyboard is read through here. Either way nothing is held, and a
         * controller that is there with nothing pressed is still a truthful
         * answer: a title asking is told about a port that answers rather than
         * one that is empty.
         *
         * Reading what is held on a desktop spoken to directly means asking it
         * for a seat and a keyboard and reading a key map, which is a piece of
         * work of its own and is not done yet. Until it is, a run wanting to
         * press something says so with XENOLITH_HOLD. */
        return 1;
    }

    char keyboard[32];
    XQueryKeymap(screen.server, keyboard);

    for (unsigned int at = 0; at < sizeof keys_to_buttons / sizeof *keys_to_buttons; at++) {
        if (held(keyboard, keys_to_buttons[at].key)) {
            *buttons |= keys_to_buttons[at].button;
        }
    }
    for (unsigned int at = 0; at < sizeof keys_to_thumbs / sizeof *keys_to_thumbs; at++) {
        if (held(keyboard, keys_to_thumbs[at].key)) {
            thumbs[keys_to_thumbs[at].which] = keys_to_thumbs[at].towards;
        }
    }
    return 1;
#else
    /* A window the desktop made its own way reports the keys it saw, which are
     * answered above. One made the older way is read through the display
     * server, which is only there where X11 is. Without it nothing is held. */
    return 1;
#endif
}

#else

int xenolith_what_is_held(uint16_t *buttons, int16_t *thumbs) {
    (void)buttons;
    (void)thumbs;
    return 0;
}

#endif

/* Keeps drawing inside the window a title named.
 *
 * A title says this in the corner it starts at and the corner it stops at,
 * both in the pixels of what is being drawn into. A window wider than what is
 * drawn into is how a title says it wants the whole of it, so the two corners
 * are brought back inside rather than handed over as they arrived: a host
 * refuses a window reaching past the edge where the console simply stopped at
 * it.
 */
void xenolith_clip_to_inside(uint32_t left, uint32_t top, uint32_t right, uint32_t bottom) {
    VkRect2D kept = {0};

    if (!recording) {
        return;
    }
    if (left > XENOLITH_GRID_ACROSS) {
        left = XENOLITH_GRID_ACROSS;
    }
    if (top > XENOLITH_GRID_DOWN) {
        top = XENOLITH_GRID_DOWN;
    }
    if (right > XENOLITH_GRID_ACROSS) {
        right = XENOLITH_GRID_ACROSS;
    }
    if (bottom > XENOLITH_GRID_DOWN) {
        bottom = XENOLITH_GRID_DOWN;
    }

    kept.offset.x = (int32_t)left;
    kept.offset.y = (int32_t)top;
    kept.extent.width = right > left ? right - left : 0;
    kept.extent.height = bottom > top ? bottom - top : 0;
    the_surface_reaches(bottom);
    last_kept = kept;
    kept_given = 1;
    vkCmdSetScissor(surface.commands, 0, 1, &kept);
}

/* Draws what a title asked to draw, with the shaders it last handed over. */
int xenolith_draw_now_inside(uint32_t blending, uint32_t written, uint32_t how_far,
                             uint32_t cut_off, uint32_t cut_off_at, uint32_t vertex_sum,
                             const uint32_t *vertex, uint32_t vertex_length, uint32_t pixel_sum,
                             const uint32_t *pixel, uint32_t pixel_length, uint32_t vertices,
                             uint32_t kind) {
    if (!recording || vertices == 0) {
        return 0;
    }

    VkPipeline with = pipeline_for(blending, written, how_far, cut_off, cut_off_at, vertex_sum,
                                   vertex, vertex_length,
                                   pixel_sum,
                                   pixel, pixel_length, kind);
    if (with == VK_NULL_HANDLE) {
        return 0;
    }

    /* A set of this draw's own, holding where in the two arenas its words were
     * put and which pictures were handed over for it. Saying it again in the
     * one a draw already recorded would change what that draw reads. */
    through.saying_now = current_lot * XENOLITH_SAYINGS + sayings_taken;
    sayings_taken++;
    say_which_pictures_are_looked_at();

    VkDescriptorBufferInfo pointing[2] = {{0}, {0}};
    pointing[0].buffer = through.constants;
    pointing[0].offset = through.constant_from;
    pointing[0].range = XENOLITH_CONSTANT_BYTES;
    VkDeviceSize share = (VkDeviceSize)current_lot * XENOLITH_VERTEX_ROOM;
    pointing[1].buffer = through.vertices;
    pointing[1].offset = share;
    pointing[1].range = through.vertex_from + through.vertex_wide - share;

    VkWriteDescriptorSet saying[2] = {{0}, {0}};
    saying[0].sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET;
    saying[0].dstSet = through.handed[through.saying_now];
    saying[0].dstBinding = 0;
    saying[0].descriptorCount = 1;
    saying[0].descriptorType = VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER;
    saying[0].pBufferInfo = &pointing[0];
    saying[1].sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET;
    saying[1].dstSet = through.handed[through.saying_now];
    saying[1].dstBinding = 1;
    saying[1].descriptorCount = 1;
    saying[1].descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER;
    saying[1].pBufferInfo = &pointing[1];
    vkUpdateDescriptorSets(drawing.device, 2, saying, 0, NULL);

    vkCmdBindPipeline(surface.commands, VK_PIPELINE_BIND_POINT_GRAPHICS, with);
    vkCmdBindDescriptorSets(surface.commands, VK_PIPELINE_BIND_POINT_GRAPHICS, through.layout, 0,
                            1, &through.handed[through.saying_now], 0, NULL);
    /* A host counts the distance added outright in the smallest step its
     * depth can take, which for depths from a half to the whole of the
     * range is one part in two to the twenty fourth. */
    vkCmdSetDepthBias(surface.commands, through.pushed_outright * 16777216.0f, 0.0f,
                      through.pushed_along_a_slope / 16.0f);
    if (through.numbers != 0) {
        vkCmdBindIndexBuffer(surface.commands, through.vertices,
                             through.vertex_from + (VkDeviceSize)through.numbers_at * 4u,
                             VK_INDEX_TYPE_UINT32);
        vkCmdDrawIndexed(surface.commands, through.numbers, 1, 0, 0, 0);
    } else {
        vkCmdDraw(surface.commands, vertices, 1, through.first_number, 0);
    }
    through.numbers = 0;
    through.first_number = 0;
    anything_recorded = 1;
    return 1;
}

/* Finishes a frame and waits for it to be drawn. */
static int xenolith_end_a_frame_inside(void) {
    if (!recording) {
        return 0;
    }
    recording = 0;

    vkCmdEndRenderPass(surface.commands);

    VkImageMemoryBarrier to_read = {0};
    to_read.sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER;
    to_read.oldLayout = VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL;
    to_read.newLayout = VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
    to_read.srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
    to_read.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
    to_read.image = surface.image;
    to_read.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
    to_read.subresourceRange.levelCount = 1;
    to_read.subresourceRange.layerCount = 1;
    to_read.srcAccessMask = VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT;
    to_read.dstAccessMask = VK_ACCESS_TRANSFER_READ_BIT;
    vkCmdPipelineBarrier(surface.commands, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                         VK_PIPELINE_STAGE_TRANSFER_BIT, 0, 0, NULL, 0, NULL, 1, &to_read);
    /* From here the surface is somewhere a copy reads from, which is what the
     * next frame's barrier has to say it is. It is not handed over here: what
     * comes after it goes into the same lot, and the lot is handed over when
     * something needs what it made.
     *
     * The frame is drawn, but not shown here. Showing every
     * frame the moment it finishes shows the ones a title copies out partway
     * through drawing as well as the one it means to display, which is what made
     * the window flash. What reaches the screen is chosen at the swap instead:
     * the frame a title copied out, or this surface as a fallback. */
    surface.read_out = 1;
    return 1;
}

/* Shows whatever is on the drawing surface now. The swap falls back to this for
 * a title that does not copy its frame out before asking for it to be shown. */
int xenolith_show_the_frame(void) {
    xenolith_send_what_was_drawn();
    __atomic_fetch_add(&frames_shown, 1ul, __ATOMIC_RELAXED);
    show_what_was_drawn(surface.image);
    return 1;
}

/* Writes what was drawn where somebody can look at it. */
/* One reading of what was drawn at a time.
 *
 * Two threads reach it: the one reading a title's commands, which copies what
 * was drawn back where the title asked for it, and whichever guest thread says
 * to keep a frame. Both look at the same memory through the same window, and a
 * window opened twice at once is not a window at all.
 */
static pthread_mutex_t one_reading = PTHREAD_MUTEX_INITIALIZER;

/* What was last copied out, and where the title said to copy it to.
 *
 * A frame is finished when a title copies it out of the hardware rather than
 * when it asks for it to be shown: it goes on drawing into the same place
 * afterwards, for things that are not this frame at all. Showing what is there
 * at the moment of the asking therefore shows what came after the frame rather
 * than the frame. So a copy is kept as it is made, and what is shown is the one
 * the title named.
 */
/* More than one of them, because a title makes several copies between one
 * asking to be shown and the next, and the one it then names is not always the
 * last it made. Keeping only the last meant the naming missed, and what reached
 * the screen was whatever pass happened to run at the end, which is often a
 * small one or nothing at all. */
#define XENOLITH_COPIES_KEPT 4u

/* Kept on the device rather than in a host's memory.
 *
 * What a device lets a host read a drawing out of is read at about a fiftieth
 * of the speed a device copies one picture to another, and a frame read back
 * twice leaves a title too little of its own frame to build a world in. Nothing
 * here looks at what a copy holds, only shows it again, so it never needs to be
 * anywhere a host can read at all. */
static struct {
    VkImage image;
    VkDeviceMemory memory;
    uint32_t where;
    unsigned long long when;
} copies[XENOLITH_COPIES_KEPT];
static unsigned long long copies_made;

/* Makes somewhere on the device to keep one, the first time it is wanted. */
static int somewhere_to_keep_a_copy(unsigned int slot) {
    if (copies[slot].image != VK_NULL_HANDLE) {
        return 1;
    }

    VkImageCreateInfo wanted = {0};
    wanted.sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO;
    wanted.imageType = VK_IMAGE_TYPE_2D;
    wanted.format = VK_FORMAT_R8G8B8A8_UNORM;
    wanted.extent.width = XENOLITH_ACROSS;
    wanted.extent.height = XENOLITH_DOWN;
    wanted.extent.depth = 1;
    wanted.mipLevels = 1;
    wanted.arrayLayers = 1;
    wanted.samples = VK_SAMPLE_COUNT_1_BIT;
    wanted.tiling = VK_IMAGE_TILING_OPTIMAL;
    wanted.usage = VK_IMAGE_USAGE_TRANSFER_SRC_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT;
    wanted.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
    if (vkCreateImage(drawing.device, &wanted, NULL, &copies[slot].image) != VK_SUCCESS) {
        copies[slot].image = VK_NULL_HANDLE;
        return 0;
    }

    VkMemoryRequirements needs;
    vkGetImageMemoryRequirements(drawing.device, copies[slot].image, &needs);

    VkMemoryAllocateInfo asking = {0};
    asking.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
    asking.allocationSize = needs.size;
    asking.memoryTypeIndex =
        memory_that_suits(needs.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);
    if (vkAllocateMemory(drawing.device, &asking, NULL, &copies[slot].memory) != VK_SUCCESS
        || vkBindImageMemory(drawing.device, copies[slot].image, copies[slot].memory, 0)
               != VK_SUCCESS) {
        vkDestroyImage(drawing.device, copies[slot].image, NULL);
        copies[slot].image = VK_NULL_HANDLE;
        return 0;
    }
    return 1;
}

/* Which of them holds this place, or nothing where none does. */
static unsigned int which_copy_holds(uint32_t where) {
    for (unsigned int at = 0; at < XENOLITH_COPIES_KEPT; at++) {
        if (copies[at].image != VK_NULL_HANDLE && copies[at].where == where) {
            return at + 1u;
        }
    }
    return 0;
}

/* Which of the copies kept a frame copied out goes into, or none.
 *
 * Only the largest a title copies out. A title copies its small drawings out as
 * well as its frames, a dozen of them between one frame and the next, and each
 * is the size it is rather than the size of a screen. Kept here they stand for
 * a whole frame that was never drawn and push out the frame a title goes on to
 * name, which left the swap with nothing to show and the screen holding
 * whatever had been drawn since.
 *
 * Which size a frame is comes from the title rather than from the surface,
 * because a title need not draw the whole of what a console shows. */
static unsigned int a_copy_to_keep(uint32_t where, uint32_t across, uint32_t down) {
    static uint32_t a_frame_is_at_least_across;
    static uint32_t a_frame_is_at_least_down;
    if (across < a_frame_is_at_least_across || down < a_frame_is_at_least_down) {
        return XENOLITH_COPIES_KEPT;
    }
    a_frame_is_at_least_across = across;
    a_frame_is_at_least_down = down;

    /* The one already standing for this place, or the one longest unused. */
    unsigned int found = which_copy_holds(where);
    unsigned int slot = found != 0 ? found - 1u : 0u;
    if (found == 0) {
        for (unsigned int at = 1; at < XENOLITH_COPIES_KEPT; at++) {
            if (copies[at].image == VK_NULL_HANDLE || copies[at].when < copies[slot].when) {
                slot = at;
            }
        }
    }
    return somewhere_to_keep_a_copy(slot) ? slot : XENOLITH_COPIES_KEPT;
}

int xenolith_show_what_was_copied_to(uint32_t where) {
    unsigned int found = where != 0 ? which_copy_holds(where) : 0u;
    if (!surface.made || found == 0) {
        return 0;
    }
    xenolith_send_what_was_drawn();
    /* Shown from where it is kept rather than put back into the surface
     * first. It is only the frame the title named for as long as nothing else
     * has been drawn, and the surface is what everything else draws into. */
    __atomic_fetch_add(&frames_shown, 1ul, __ATOMIC_RELAXED);
    show_what_was_drawn(copies[found - 1u].image);
    return 1;
}

/* Forgets any picture already turned that was made out of memory a title has
 * since written over.
 *
 * A title draws into memory and then reads that memory as a picture, which is
 * how it builds the ones it makes rather than loads: a page of letters, a
 * reflection, a frame it means to blur. Keeping what was turned the first time
 * hands every later draw the first thing that memory ever held, so the picture
 * a title just drew is never the one it sees.
 *
 * What is written over is kept and marked so rather than let go of, because
 * the next copy into it can usually be put into it on the device, and it is
 * turned again only if it is read before then.
 */
void xenolith_forget_pictures_at(uint32_t where, uint32_t bytes) {
    if (!surface.made || bytes == 0) {
        return;
    }

    for (uint32_t at = 0; at < XENOLITH_PICTURES_KEPT; at++) {
        /* Any picture the memory written reaches into, rather than only those
         * starting inside it. A frame copied out in parts writes the later
         * parts part of the way into a picture that starts with the first. */
        uint32_t ends = turned[at].from
                        + (turned[at].over != 0 ? turned[at].over
                                                : turned[at].across * turned[at].down * 4u);
        if (!turned[at].held || ends <= where || turned[at].from >= where + bytes
            || turned[at].renewed_by == copies_counted) {
            continue;
        }

        for (uint32_t place = 0; place < XENOLITH_PICTURES; place++) {
            if (pictures[place] == at + 1u) {
                pictures[place] = 0;
            }
        }
        turned[at].stale = 1;
    }
}

void xenolith_look_at_nothing(uint32_t slot) {
    if (slot < XENOLITH_PICTURES) {
        pictures[slot] = 0;
    }
}

/* One of the surface's half floats as a whole one. */
static float a_half(const unsigned char *at) {
    uint16_t half;
    memcpy(&half, at, sizeof half);
    uint32_t sign = (uint32_t)(half >> 15) << 31;
    uint32_t exponent = (half >> 10) & 0x1fu;
    uint32_t fraction = half & 0x3ffu;
    uint32_t word;
    if (exponent == 0) {
        float small = ldexpf((float)fraction, -24);
        return sign != 0 ? -small : small;
    }
    if (exponent == 0x1fu) {
        word = sign | 0x7f800000u | (fraction << 13);
    } else {
        word = sign | ((exponent + 112u) << 23) | (fraction << 13);
    }
    float whole;
    memcpy(&whole, &word, sizeof whole);
    return whole;
}

/* A colour's part as a byte, held between nought and one first. */
static uint32_t a_part_as_a_byte(float part) {
    if (!(part > 0.0f)) {
        return 0u;
    }
    if (part >= 1.0f) {
        return 255u;
    }
    return (uint32_t)(part * 255.0f + 0.5f);
}

/* Every half float as a whole one and as a byte, worked out once.
 *
 * A copy turns every part of every pixel it takes, a few million of them for
 * each frame a title copies out, and working each out on its own came to most
 * of what a copy cost. There are only sixty five thousand of them. */
static float halves_as_floats[65536];
static uint8_t halves_as_bytes[65536];

static void work_out_the_halves(void) {
    static int worked_out;
    if (worked_out) {
        return;
    }
    for (uint32_t one = 0; one < 65536u; one++) {
        uint16_t half = (uint16_t)one;
        unsigned char at[2];
        memcpy(at, &half, sizeof at);
        halves_as_floats[one] = a_half(at);
        halves_as_bytes[one] = (uint8_t)a_part_as_a_byte(halves_as_floats[one]);
    }
    worked_out = 1;
}

/* Turns how far away a thing is into the console's small float, four bits of
 * exponent over twenty of mantissa, which is the other way it keeps one. */
static uint32_t a_short_float_from_how_far(float away) {
    if (!(away > 0.0f)) {
        return 0u;
    }
    int twos = 0;
    float part = frexpf(away, &twos);
    /* frexp hands back a half and up; the console's is a one and up. */
    int exponent = twos - 1 + 15;
    float mantissa = part * 2.0f - 1.0f;
    if (exponent <= 0) {
        uint32_t small = (uint32_t)(ldexpf(away, 14) * 1048576.0f);
        return small > 0xfffffu ? 0xfffffu : small;
    }
    if (exponent > 15) {
        return 0xffffffu;
    }
    uint32_t held = (uint32_t)(mantissa * 1048576.0f);
    if (held > 0xfffffu) {
        held = 0xfffffu;
    }
    return ((uint32_t)exponent << 20) | held;
}

/* Where a copy out of what was drawn is made into bytes on the device, one
 * with red first and one with blue first, on its way into a picture. */
#define XENOLITH_TURNING_ACROSS 2048u
#define XENOLITH_TURNING_DOWN 2048u
#define XENOLITH_RENEWED_AT_ONCE 16u

static struct {
    VkImage image;
    VkDeviceMemory memory;
} turning[2];

static int somewhere_to_turn_a_copy(int blue_first) {
    if (turning[blue_first].image != VK_NULL_HANDLE) {
        return 1;
    }
    VkImageCreateInfo wanted = {0};
    wanted.sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO;
    wanted.imageType = VK_IMAGE_TYPE_2D;
    wanted.format = blue_first ? VK_FORMAT_B8G8R8A8_UNORM : VK_FORMAT_R8G8B8A8_UNORM;
    wanted.extent.width = XENOLITH_TURNING_ACROSS;
    wanted.extent.height = XENOLITH_TURNING_DOWN;
    wanted.extent.depth = 1;
    wanted.mipLevels = 1;
    wanted.arrayLayers = 1;
    wanted.samples = VK_SAMPLE_COUNT_1_BIT;
    wanted.tiling = VK_IMAGE_TILING_OPTIMAL;
    wanted.usage = VK_IMAGE_USAGE_TRANSFER_SRC_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT
                   | VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT;
    wanted.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
    VkImage made = VK_NULL_HANDLE;
    if (vkCreateImage(drawing.device, &wanted, NULL, &made) != VK_SUCCESS) {
        return 0;
    }
    VkMemoryRequirements needs;
    vkGetImageMemoryRequirements(drawing.device, made, &needs);
    VkMemoryAllocateInfo asking = {0};
    asking.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO;
    asking.allocationSize = needs.size;
    asking.memoryTypeIndex =
        memory_that_suits(needs.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);
    VkDeviceMemory its_own = VK_NULL_HANDLE;
    if (vkAllocateMemory(drawing.device, &asking, NULL, &its_own) != VK_SUCCESS) {
        vkDestroyImage(drawing.device, made, NULL);
        return 0;
    }
    if (vkBindImageMemory(drawing.device, made, its_own, 0) != VK_SUCCESS) {
        vkFreeMemory(drawing.device, its_own, NULL);
        vkDestroyImage(drawing.device, made, NULL);
        return 0;
    }
    turning[blue_first].image = made;
    turning[blue_first].memory = its_own;
    return 1;
}

/* Where in a picture already turned a copy lands, or nothing where it does not
 * land in it as the picture stands.
 *
 * Only a picture of one of the two kinds asked for, kept in squares, four bytes
 * to a part turned word by word, in runs as wide as the ones the copy writes.
 * What the copy writes past the picture's edge is left out, which is how much
 * of it lands is given too: a title halving a scene down to one reading copies
 * a rectangle eight wide into a picture four wide. A square there is thirty two parts on a side and four kilobytes,
 * laid a run of them at a time, so where in it the copy starts says which. */
static int where_a_copy_lands(uint32_t which, const struct xenolith_copy *copy,
                              uint32_t right, uint32_t bottom, uint32_t kept_as,
                              uint32_t or_as, uint32_t *across_at, uint32_t *down_at,
                              uint32_t *lands_right, uint32_t *lands_bottom) {
    uint32_t along = (copy->pitch + 31u) & ~31u;
    if (!turned[which].held
        || (turned[which].kept != kept_as && turned[which].kept != or_as)
        || !turned[which].in_squares || turned[which].order != 2u
        || turned[which].along != along || copy->into < turned[which].from
        || copy->into - turned[which].from >= turned[which].over
        || (copy->into - turned[which].from) % XENOLITH_WHAT_A_SQUARE_HOLDS != 0) {
        return 0;
    }
    uint32_t square = (copy->into - turned[which].from) / XENOLITH_WHAT_A_SQUARE_HOLDS;
    uint32_t squares_a_run = along / XENOLITH_A_SQUARE_ACROSS;
    *across_at = (square % squares_a_run) * XENOLITH_A_SQUARE_ACROSS;
    *down_at = (square / squares_a_run) * XENOLITH_A_SQUARE_ACROSS;
    if (*across_at >= turned[which].across || *down_at >= turned[which].down) {
        return 0;
    }
    uint32_t room_across = turned[which].across - *across_at;
    uint32_t room_down = turned[which].down - *down_at;
    *lands_right = right < room_across ? right : room_across;
    *lands_bottom = bottom < room_down ? bottom : room_down;
    return *lands_right > copy->left && *lands_bottom > copy->top;
}

/* Makes what a copy takes into bytes on the device, the way round the copy
 * writes them, blue first where it swaps red and blue. Where each pixel was
 * drawn as more than one sample they are averaged on the way.
 *
 * Done on the host, turning every sample from a half float and averaging it
 * came to most of what a frame of a mission cost, for the few dozen copies it
 * makes. */
static VkImage a_copy_made_into_bytes(VkCommandBuffer one, const struct xenolith_copy *copy,
                                      const VkBufferImageCopy *out, uint32_t across,
                                      uint32_t down) {
    int blue_first = copy->red_and_blue_swapped != 0;
    if (across > XENOLITH_TURNING_ACROSS || down > XENOLITH_TURNING_DOWN
        || !somewhere_to_turn_a_copy(blue_first)) {
        return VK_NULL_HANDLE;
    }
    VkImage bytes = turning[blue_first].image;
    a_picture_becomes(one, bytes, VK_IMAGE_LAYOUT_UNDEFINED, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL);
    VkImageBlit made = {0};
    made.srcSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
    made.srcSubresource.layerCount = 1;
    made.srcOffsets[0] = out->imageOffset;
    made.srcOffsets[1].x = out->imageOffset.x + (int32_t)out->imageExtent.width;
    made.srcOffsets[1].y = out->imageOffset.y + (int32_t)out->imageExtent.height;
    made.srcOffsets[1].z = 1;
    made.dstSubresource = made.srcSubresource;
    made.dstOffsets[1].x = (int32_t)across;
    made.dstOffsets[1].y = (int32_t)down;
    made.dstOffsets[1].z = 1;
    vkCmdBlitImage(one, surface.image, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, bytes,
                   VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1, &made, VK_FILTER_LINEAR);
    a_picture_becomes(one, bytes, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                      VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL);
    return bytes;
}

/* Puts a copy already made into bytes where the title asked for it, in
 * squares.
 *
 * Four parts side by side in a square are four words side by side in memory,
 * so they go across together, each word turned round to the larger end first,
 * and where they go is worked out once for the four. */
static void put_a_copy_in_squares(uint8_t *base, const struct xenolith_copy *copy,
                                  const unsigned char *from, uint32_t right,
                                  uint32_t squares_a_row, uint32_t first_row,
                                  uint32_t last_row) {
    uint32_t across = right - copy->left;
    for (uint32_t row = first_row; row < last_row; row++) {
        uint32_t down_to = (row / XENOLITH_A_SQUARE_ACROSS) * squares_a_row
                           * XENOLITH_WHAT_A_SQUARE_HOLDS;
        const uint32_t *in_the_square =
            &where_in_a_square[(row % XENOLITH_A_SQUARE_ACROSS) * XENOLITH_A_SQUARE_ACROSS];
        const unsigned char *line = from + (size_t)(row - copy->top) * across * 4u;
        uint32_t at = copy->left;
        while (at < right) {
            uint32_t run = 4u - (at & 3u);
            if (run > right - at) {
                run = right - at;
            }
            uint8_t *to = base + copy->into + down_to
                          + (at / XENOLITH_A_SQUARE_ACROSS) * XENOLITH_WHAT_A_SQUARE_HOLDS
                          + in_the_square[at % XENOLITH_A_SQUARE_ACROSS];
            const unsigned char *was = line + (size_t)(at - copy->left) * 4u;
            for (uint32_t byte = 0; byte < run * 4u; byte += 4u) {
                to[byte] = was[byte + 3u];
                to[byte + 1u] = was[byte + 2u];
                to[byte + 2u] = was[byte + 1u];
                to[byte + 3u] = was[byte];
            }
            at += run;
        }
    }
}

/* Puts a copy made into bytes into every picture already turned out of the
 * memory it writes, and says which pictures those were.
 *
 * A title copies a scene out and then reads it as a picture. Turning that
 * picture again out of the memory the copy wrote reads back what the device
 * already holds, puts it in order and hands it over again, which in a mission
 * came to a third of the time a frame took. */
static uint32_t renew_the_pictures_a_copy_writes(VkCommandBuffer one, VkImage bytes,
                                                 const struct xenolith_copy *copy, uint32_t right,
                                                 uint32_t bottom, uint32_t *renewed) {
    uint32_t count = 0;
    for (uint32_t which = 0; which < XENOLITH_PICTURES_KEPT && count < XENOLITH_RENEWED_AT_ONCE;
         which++) {
        uint32_t across_at = 0;
        uint32_t down_at = 0;
        uint32_t lands_right = 0;
        uint32_t lands_bottom = 0;
        if (!where_a_copy_lands(which, copy, right, bottom, XENOLITH_PICTURE_IN_FOURS,
                                XENOLITH_PICTURE_IN_FOURS, &across_at, &down_at, &lands_right,
                                &lands_bottom)) {
            continue;
        }
        a_picture_becomes(one, turned[which].image, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                          VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL);
        VkImageCopy putting = {0};
        putting.srcSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
        putting.srcSubresource.layerCount = 1;
        putting.dstSubresource = putting.srcSubresource;
        putting.dstOffset.x = (int32_t)(across_at + copy->left);
        putting.dstOffset.y = (int32_t)(down_at + copy->top);
        putting.extent.width = lands_right - copy->left;
        putting.extent.height = lands_bottom - copy->top;
        putting.extent.depth = 1;
        vkCmdCopyImage(one, bytes, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, turned[which].image,
                       VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1, &putting);
        a_picture_becomes(one, turned[which].image, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                          VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL);
        renewed[count++] = which;
    }
    return count;
}

/* Hands what was drawn back to the title, where the title said to put it.
 *
 * A console draws into memory of its own beside the hardware and then copies
 * that out into memory a title can reach, saying where, how wide a row is, and
 * how tall. What a title does next is its own business: it reads that memory,
 * or shows it, or draws over it. Without the copy it reads whatever was there,
 * which is nothing, however right what was drawn was.
 *
 * Only the rectangle the title named is read back and written. A title drawing
 * a frame in parts copies each part into its own share of the one picture, and
 * writing the whole of what was drawn each time wrote every part over the ones
 * before it.
 *
 * The colours go out the way round the hardware keeps them, red last, unless
 * the copy asks for red and blue to be swapped, which a title asks for where it
 * means to show what it copies.
 *
 * It goes out in squares rather than row by row, because that is the shape a
 * title reads it back in. The console copies into a picture, and a picture is
 * kept in squares, so a title that copies a scene out and then draws with it
 * hands the same squared reading to the hardware either way. Written row by
 * row it came back through that reading scrambled, which is how a scene drawn
 * right ended up as a checkerboard behind everything else.
 */
/* Puts a copy of one float to a pixel into every picture of floats already
 * turned out of the memory it writes, straight from what was drawn.
 *
 * A title works out how bright a scene is, and how far away each part of it
 * is, as one float to a pixel, and reads each straight back as a picture.
 * Turned again from memory every time, each of those waited for everything
 * recorded before it to run. A blit takes the first part of each pixel and
 * averages its samples, which is what the copy writes to memory. */
static uint32_t renew_the_floats_a_copy_writes(VkCommandBuffer one,
                                               const struct xenolith_copy *copy,
                                               const VkBufferImageCopy *out, uint32_t right,
                                               uint32_t bottom, uint32_t *renewed) {
    uint32_t count = 0;
    for (uint32_t which = 0; which < XENOLITH_PICTURES_KEPT && count < XENOLITH_RENEWED_AT_ONCE;
         which++) {
        uint32_t across_at = 0;
        uint32_t down_at = 0;
        uint32_t lands_right = 0;
        uint32_t lands_bottom = 0;
        if (!where_a_copy_lands(which, copy, right, bottom, XENOLITH_PICTURE_IN_FLOATS,
                                XENOLITH_PICTURE_IN_FLOATS, &across_at, &down_at, &lands_right,
                                &lands_bottom)) {
            continue;
        }
        uint32_t across_by = out->imageExtent.width / (right - copy->left);
        uint32_t down_by = out->imageExtent.height / (bottom - copy->top);
        a_picture_becomes(one, turned[which].image, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                          VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL);
        VkImageBlit putting = {0};
        putting.srcSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
        putting.srcSubresource.layerCount = 1;
        putting.srcOffsets[0] = out->imageOffset;
        putting.srcOffsets[1].x =
            out->imageOffset.x + (int32_t)((lands_right - copy->left) * across_by);
        putting.srcOffsets[1].y =
            out->imageOffset.y + (int32_t)((lands_bottom - copy->top) * down_by);
        putting.srcOffsets[1].z = 1;
        putting.dstSubresource = putting.srcSubresource;
        putting.dstOffsets[0].x = (int32_t)(across_at + copy->left);
        putting.dstOffsets[0].y = (int32_t)(down_at + copy->top);
        putting.dstOffsets[1].x = (int32_t)(across_at + lands_right);
        putting.dstOffsets[1].y = (int32_t)(down_at + lands_bottom);
        putting.dstOffsets[1].z = 1;
        vkCmdBlitImage(one, surface.image, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                       turned[which].image, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1, &putting,
                       VK_FILTER_LINEAR);
        a_picture_becomes(one, turned[which].image, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                          VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL);
        renewed[count++] = which;
    }
    return count;
}

/* Puts a copy of how far away things are into every picture of distances
 * already turned out of the memory it writes, from where it was read back.
 *
 * What was read back holds every sample, and the copy writes the first of each
 * pixel's, so the rows of the read back are walked a pixel's samples at a time.
 * That only lines up where a pixel's samples are one above the other; one with
 * two side by side is left to be turned again. */
static uint32_t renew_the_distances_a_copy_writes(VkCommandBuffer one,
                                                  const struct xenolith_copy *copy,
                                                  VkDeviceSize from, uint32_t right,
                                                  uint32_t bottom, uint32_t *renewed) {
    uint32_t across_by = copy->samples_across != 0 ? copy->samples_across : 1u;
    uint32_t down_by = copy->samples_down != 0 ? copy->samples_down : 1u;
    if (across_by != 1u) {
        return 0;
    }
    uint32_t count = 0;
    int read_back_is_ready = 0;
    for (uint32_t which = 0; which < XENOLITH_PICTURES_KEPT && count < XENOLITH_RENEWED_AT_ONCE;
         which++) {
        uint32_t across_at = 0;
        uint32_t down_at = 0;
        uint32_t lands_right = 0;
        uint32_t lands_bottom = 0;
        if (!where_a_copy_lands(which, copy, right, bottom, XENOLITH_PICTURE_HOW_FAR,
                                XENOLITH_PICTURE_HOW_FAR_AS_A_FLOAT, &across_at, &down_at,
                                &lands_right, &lands_bottom)) {
            continue;
        }
        if (!read_back_is_ready) {
            VkBufferMemoryBarrier written = {0};
            written.sType = VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER;
            written.srcAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;
            written.dstAccessMask = VK_ACCESS_TRANSFER_READ_BIT;
            written.srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
            written.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
            written.buffer = surface.readable;
            written.offset = from;
            written.size = VK_WHOLE_SIZE;
            vkCmdPipelineBarrier(one, VK_PIPELINE_STAGE_TRANSFER_BIT,
                                 VK_PIPELINE_STAGE_TRANSFER_BIT, 0, 0, NULL, 1, &written, 0,
                                 NULL);
            read_back_is_ready = 1;
        }
        a_picture_becomes(one, turned[which].image, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                          VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL);
        VkBufferImageCopy putting = {0};
        putting.bufferOffset = from;
        putting.bufferRowLength = (right - copy->left) * down_by;
        putting.imageSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
        putting.imageSubresource.layerCount = 1;
        putting.imageOffset.x = (int32_t)(across_at + copy->left);
        putting.imageOffset.y = (int32_t)(down_at + copy->top);
        putting.imageExtent.width = lands_right - copy->left;
        putting.imageExtent.height = lands_bottom - copy->top;
        putting.imageExtent.depth = 1;
        vkCmdCopyBufferToImage(one, surface.readable, turned[which].image,
                               VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1, &putting);
        a_picture_becomes(one, turned[which].image, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                          VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL);
        renewed[count++] = which;
    }
    return count;
}

/* What turning how far away things are into bytes takes: somewhere to draw
 * the bytes, the one way of drawing there, and the pipeline that does it. */
static struct {
    VkRenderPass pass;
    VkImageView seen;
    VkFramebuffer target;
    VkPipelineLayout layout;
    VkPipeline pipeline;
    int made;
    int refused;
} turning_distances;

static int make_what_turning_distances_takes(void) {
    if (turning_distances.made || turning_distances.refused) {
        return turning_distances.made;
    }
    turning_distances.refused = 1;
    if (!make_what_moving_round_takes() || !somewhere_to_turn_a_copy(0)) {
        return 0;
    }

    VkAttachmentDescription bytes = {0};
    bytes.format = VK_FORMAT_R8G8B8A8_UNORM;
    bytes.samples = VK_SAMPLE_COUNT_1_BIT;
    bytes.loadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE;
    bytes.storeOp = VK_ATTACHMENT_STORE_OP_STORE;
    bytes.stencilLoadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE;
    bytes.stencilStoreOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
    bytes.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
    bytes.finalLayout = VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
    VkAttachmentReference drawn_into = {0, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL};
    VkSubpassDescription only = {0};
    only.pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
    only.colorAttachmentCount = 1;
    only.pColorAttachments = &drawn_into;
    VkSubpassDependency around[2] = {{0}, {0}};
    around[0].srcSubpass = VK_SUBPASS_EXTERNAL;
    around[0].dstSubpass = 0;
    around[0].srcStageMask = VK_PIPELINE_STAGE_TRANSFER_BIT;
    around[0].dstStageMask = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
    around[0].dstAccessMask = VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT;
    around[1].srcSubpass = 0;
    around[1].dstSubpass = VK_SUBPASS_EXTERNAL;
    around[1].srcStageMask = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
    around[1].dstStageMask = VK_PIPELINE_STAGE_TRANSFER_BIT;
    around[1].srcAccessMask = VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT;
    around[1].dstAccessMask = VK_ACCESS_TRANSFER_READ_BIT;
    VkRenderPassCreateInfo passing = {0};
    passing.sType = VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO;
    passing.attachmentCount = 1;
    passing.pAttachments = &bytes;
    passing.subpassCount = 1;
    passing.pSubpasses = &only;
    passing.dependencyCount = 2;
    passing.pDependencies = around;
    if (vkCreateRenderPass(drawing.device, &passing, NULL, &turning_distances.pass)
        != VK_SUCCESS) {
        return 0;
    }

    VkImageViewCreateInfo seeing = {0};
    seeing.sType = VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO;
    seeing.image = turning[0].image;
    seeing.viewType = VK_IMAGE_VIEW_TYPE_2D;
    seeing.format = VK_FORMAT_R8G8B8A8_UNORM;
    seeing.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
    seeing.subresourceRange.levelCount = 1;
    seeing.subresourceRange.layerCount = 1;
    if (vkCreateImageView(drawing.device, &seeing, NULL, &turning_distances.seen) != VK_SUCCESS) {
        return 0;
    }
    VkFramebufferCreateInfo framing = {0};
    framing.sType = VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO;
    framing.renderPass = turning_distances.pass;
    framing.attachmentCount = 1;
    framing.pAttachments = &turning_distances.seen;
    framing.width = XENOLITH_TURNING_ACROSS;
    framing.height = XENOLITH_TURNING_DOWN;
    framing.layers = 1;
    if (vkCreateFramebuffer(drawing.device, &framing, NULL, &turning_distances.target)
        != VK_SUCCESS) {
        return 0;
    }

    VkPushConstantRange how = {VK_SHADER_STAGE_FRAGMENT_BIT, 0, 3u * sizeof(int32_t)};
    VkPipelineLayoutCreateInfo outside = {0};
    outside.sType = VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO;
    outside.setLayoutCount = 1;
    outside.pSetLayouts = &rearranging.shape;
    outside.pushConstantRangeCount = 1;
    outside.pPushConstantRanges = &how;
    if (vkCreatePipelineLayout(drawing.device, &outside, NULL, &turning_distances.layout)
        != VK_SUCCESS) {
        return 0;
    }
    turning_distances.pipeline =
        a_pipeline_of_our_own(distances_into_bytes, sizeof distances_into_bytes / 4u,
                              turning_distances.layout, turning_distances.pass, 0);
    if (turning_distances.pipeline == VK_NULL_HANDLE) {
        return 0;
    }
    turning_distances.made = 1;
    turning_distances.refused = 0;
    return 1;
}

/* Puts a copy of how far away things are into every picture of four bytes to a
 * part already turned out of the memory it writes, as the bytes the copy
 * writes there, and says which pictures those were.
 *
 * A title copies how far away its scene is out and reads the same memory back
 * as a picture of bytes, to work out from them how far away each pixel was.
 * Turned again from memory, that picture waited every frame for everything
 * recorded before it to run, the copy and the whole scene before it, which
 * was the processor and the device waiting on one another again. So the
 * distances are drawn into bytes on the device and put into the picture from
 * there. What is drawn from is the first sample of each pixel, which is what
 * the copy writes. */
static uint32_t turn_distances_into_bytes(VkCommandBuffer one, const struct xenolith_copy *copy,
                                          const VkBufferImageCopy *out, uint32_t right,
                                          uint32_t bottom, uint32_t *renewed) {
    uint32_t across = right - copy->left;
    uint32_t down = bottom - copy->top;
    if (across > XENOLITH_TURNING_ACROSS || down > XENOLITH_TURNING_DOWN) {
        return 0;
    }
    uint32_t landing[XENOLITH_RENEWED_AT_ONCE];
    uint32_t across_at[XENOLITH_RENEWED_AT_ONCE];
    uint32_t down_at[XENOLITH_RENEWED_AT_ONCE];
    uint32_t lands_right[XENOLITH_RENEWED_AT_ONCE];
    uint32_t lands_bottom[XENOLITH_RENEWED_AT_ONCE];
    uint32_t count = 0;
    for (uint32_t which = 0; which < XENOLITH_PICTURES_KEPT && count < XENOLITH_RENEWED_AT_ONCE;
         which++) {
        if (where_a_copy_lands(which, copy, right, bottom, XENOLITH_PICTURE_IN_FOURS,
                               XENOLITH_PICTURE_IN_FOURS, &across_at[count], &down_at[count],
                               &lands_right[count], &lands_bottom[count])) {
            landing[count++] = which;
        }
    }
    if (count == 0 || !make_what_turning_distances_takes()) {
        return 0;
    }

    VkImage held = moving_round[1].image;
    samples_become(one, held, VK_IMAGE_ASPECT_DEPTH_BIT, VK_IMAGE_LAYOUT_UNDEFINED,
                   VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL);
    VkImageCopy taking = {0};
    taking.srcSubresource.aspectMask = VK_IMAGE_ASPECT_DEPTH_BIT;
    taking.srcSubresource.layerCount = 1;
    taking.dstSubresource = taking.srcSubresource;
    taking.srcOffset = out->imageOffset;
    taking.extent = out->imageExtent;
    vkCmdCopyImage(one, through.how_far, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, held,
                   VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1, &taking);
    samples_become(one, held, VK_IMAGE_ASPECT_DEPTH_BIT, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                   VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL);

    VkRenderPassBeginInfo into = {0};
    into.sType = VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO;
    into.renderPass = turning_distances.pass;
    into.framebuffer = turning_distances.target;
    into.renderArea.extent.width = across;
    into.renderArea.extent.height = down;
    vkCmdBeginRenderPass(one, &into, VK_SUBPASS_CONTENTS_INLINE);
    VkViewport seen = {0};
    seen.width = (float)across;
    seen.height = (float)down;
    seen.maxDepth = 1.0f;
    VkRect2D kept = {0};
    kept.extent.width = across;
    kept.extent.height = down;
    vkCmdSetViewport(one, 0, 1, &seen);
    vkCmdSetScissor(one, 0, 1, &kept);
    vkCmdBindPipeline(one, VK_PIPELINE_BIND_POINT_GRAPHICS, turning_distances.pipeline);
    vkCmdBindDescriptorSets(one, VK_PIPELINE_BIND_POINT_GRAPHICS, turning_distances.layout, 0, 1,
                            &rearranging.said[1], 0, NULL);
    int32_t how[3] = {(int32_t)(out->imageExtent.width / across),
                      (int32_t)(out->imageExtent.height / down), copy->how_far_as_a_float != 0};
    vkCmdPushConstants(one, turning_distances.layout, VK_SHADER_STAGE_FRAGMENT_BIT, 0, sizeof how,
                       how);
    vkCmdDraw(one, 3, 1, 0, 0);
    vkCmdEndRenderPass(one);

    for (uint32_t at = 0; at < count; at++) {
        uint32_t which = landing[at];
        a_picture_becomes(one, turned[which].image, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                          VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL);
        VkImageCopy putting = {0};
        putting.srcSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
        putting.srcSubresource.layerCount = 1;
        putting.dstSubresource = putting.srcSubresource;
        putting.dstOffset.x = (int32_t)(across_at[at] + copy->left);
        putting.dstOffset.y = (int32_t)(down_at[at] + copy->top);
        putting.extent.width = lands_right[at] - copy->left;
        putting.extent.height = lands_bottom[at] - copy->top;
        putting.extent.depth = 1;
        vkCmdCopyImage(one, turning[0].image, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                       turned[which].image, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1, &putting);
        a_picture_becomes(one, turned[which].image, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                          VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL);
        renewed[at] = which;
    }
    return count;
}

/* A copy recorded and waiting for the lot it is in to run, so that what it
 * read back can be put where the title asked for it. */
#define XENOLITH_WAITING_COPIES 512u

struct a_waiting_copy {
    uint8_t *base;
    struct xenolith_copy copy;
    uint32_t right;
    uint32_t bottom;
    VkDeviceSize at;
    int in_bytes;
    uint32_t renewed[XENOLITH_RENEWED_AT_ONCE * 2u];
    uint32_t renewing;
};
static struct a_waiting_copy waiting[XENOLITH_LOTS][XENOLITH_WAITING_COPIES];
static uint32_t copies_waiting[XENOLITH_LOTS];

/* How much of each lot's share of the room for handing copies back its waiting
 * copies have taken, counted from the front of the whole room. */
#define XENOLITH_HANDED_BACK_A_LOT (XENOLITH_HANDED_BACK_ROOM / XENOLITH_LOTS)
static VkDeviceSize handed_back_at[XENOLITH_LOTS];

/* Whether memory a title might read a picture from is still to be written by
 * a copy that has not run yet. */
static int a_copy_is_still_to_write(uint32_t lot, uint32_t from, uint32_t over) {
    for (uint32_t which = 0; which < copies_waiting[lot]; which++) {
        const struct xenolith_copy *copy = &waiting[lot][which].copy;
        uint32_t reaches = xenolith_what_a_copy_reaches(copy->pitch, waiting[lot][which].right,
                                                        waiting[lot][which].bottom);
        if (copy->into < from + over && from < copy->into + reaches) {
            return 1;
        }
    }
    return 0;
}

/* Hands what was drawn back to the title, where the title said to put it.
 *
 * A console draws into memory of its own beside the hardware and then copies
 * that out into memory a title can reach, saying where, how wide a row is, and
 * how tall. What a title does next is its own business: it reads that memory,
 * or shows it, or draws over it. Without the copy it reads whatever was there,
 * which is nothing, however right what was drawn was.
 *
 * What the copy does on the device is recorded here, into the lot the frame is
 * in, and the memory is written once that lot has run: it is read back into a
 * share of the room kept for it, and put in place by
 * [`put_back_what_was_copied`] when the lot is handed over.
 *
 * Only the rectangle the title named is read back and written. A title drawing
 * a frame in parts copies each part into its own share of the one picture, and
 * writing the whole of what was drawn each time wrote every part over the ones
 * before it.
 */
int xenolith_hand_back_what_was_drawn(uint8_t *base, const struct xenolith_copy *copy) {
    uint32_t right = copy->right < copy->pitch ? copy->right : copy->pitch;
    uint32_t bottom = copy->bottom < copy->height ? copy->bottom : copy->height;
    uint32_t across_by = copy->samples_across != 0 ? copy->samples_across : 1u;
    uint32_t down_by = copy->samples_down != 0 ? copy->samples_down : 1u;
    if (right > XENOLITH_GRID_ACROSS / across_by) {
        right = XENOLITH_GRID_ACROSS / across_by;
    }
    if (bottom > XENOLITH_GRID_DOWN / down_by) {
        bottom = XENOLITH_GRID_DOWN / down_by;
    }
    if (!surface.made || !surface.read_out || copy->into == 0 || copy->left >= right
        || copy->top >= bottom) {
        return 0;
    }

    uint32_t across = right - copy->left;
    uint32_t down = bottom - copy->top;
    uint32_t grid_across = across * across_by;
    uint32_t grid_down = down * down_by;
    size_t sample_bytes = copy->from_how_far ? 4u : XENOLITH_SURFACE_BYTES;
    VkDeviceSize room = (VkDeviceSize)grid_across * grid_down * sample_bytes;

    drawn_across = copy->pitch < XENOLITH_ACROSS ? copy->pitch : XENOLITH_ACROSS;
    drawn_down = copy->height < XENOLITH_DOWN ? copy->height : XENOLITH_DOWN;

    pthread_mutex_lock(&one_drawing);
    if (recording) {
        pthread_mutex_unlock(&one_drawing);
        return 0;
    }
    if (batch_open
        && (copies_waiting[current_lot] >= XENOLITH_WAITING_COPIES
            || handed_back_at[current_lot] + room
                   > (VkDeviceSize)(current_lot + 1u) * XENOLITH_HANDED_BACK_A_LOT)) {
        send_what_was_recorded();
    }
    if (!begin_the_batch()) {
        pthread_mutex_unlock(&one_drawing);
        return 0;
    }
    VkCommandBuffer one = surface.commands;
    uint32_t lot = current_lot;

    VkBufferImageCopy out = {0};
    out.bufferOffset = handed_back_at[lot];
    out.imageSubresource.aspectMask =
        copy->from_how_far ? VK_IMAGE_ASPECT_DEPTH_BIT : VK_IMAGE_ASPECT_COLOR_BIT;
    out.imageSubresource.layerCount = 1;
    out.imageOffset.x = (int32_t)(copy->left * across_by);
    out.imageOffset.y = (int32_t)(copy->top * down_by);
    out.imageExtent.width = grid_across;
    out.imageExtent.height = grid_down;
    out.imageExtent.depth = 1;
    if (!copy->from_how_far) {
        the_surface_reaches((uint32_t)out.imageOffset.y + grid_down);
    }

    struct a_waiting_copy *pending = &waiting[lot][copies_waiting[lot]];
    pending->base = base;
    pending->copy = *copy;
    pending->right = right;
    pending->bottom = bottom;
    pending->at = handed_back_at[lot];
    pending->in_bytes = 0;
    pending->renewing = 0;
    copies_counted++;

    /* How far away things came out is kept apart from their colours, and is
     * somewhere to draw rather than somewhere to read from until it is asked
     * for. */
    unsigned int kept = XENOLITH_COPIES_KEPT;
    if (copy->from_how_far) {
        VkImageMemoryBarrier to_read = {0};
        to_read.sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER;
        to_read.oldLayout = VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL;
        to_read.newLayout = VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
        to_read.srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
        to_read.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
        to_read.image = through.how_far;
        to_read.subresourceRange.aspectMask = VK_IMAGE_ASPECT_DEPTH_BIT;
        to_read.subresourceRange.levelCount = 1;
        to_read.subresourceRange.layerCount = 1;
        to_read.srcAccessMask = VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT;
        to_read.dstAccessMask = VK_ACCESS_TRANSFER_READ_BIT;
        vkCmdPipelineBarrier(one, VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
                             VK_PIPELINE_STAGE_TRANSFER_BIT, 0, 0, NULL, 0, NULL, 1, &to_read);

        vkCmdCopyImageToBuffer(one, through.how_far, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                               surface.readable, 1, &out);
        uint32_t as_bytes =
            turn_distances_into_bytes(one, copy, &out, right, bottom, pending->renewed);

        VkImageMemoryBarrier back_to_drawing = to_read;
        back_to_drawing.oldLayout = VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
        back_to_drawing.newLayout = VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL;
        back_to_drawing.srcAccessMask = VK_ACCESS_TRANSFER_READ_BIT;
        back_to_drawing.dstAccessMask = VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT;
        vkCmdPipelineBarrier(one, VK_PIPELINE_STAGE_TRANSFER_BIT,
                             VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT, 0, 0, NULL, 0, NULL, 1,
                             &back_to_drawing);
        pending->renewing =
            as_bytes + renew_the_distances_a_copy_writes(one, copy, handed_back_at[lot], right,
                                                         bottom, pending->renewed + as_bytes);
    } else {
        VkImage bytes = copy->as_one_float
                            ? VK_NULL_HANDLE
                            : a_copy_made_into_bytes(one, copy, &out, across, down);
        if (bytes != VK_NULL_HANDLE) {
            VkBufferImageCopy made = {0};
            made.bufferOffset = handed_back_at[lot];
            made.imageSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
            made.imageSubresource.layerCount = 1;
            made.imageExtent.width = across;
            made.imageExtent.height = down;
            made.imageExtent.depth = 1;
            vkCmdCopyImageToBuffer(one, bytes, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                                   surface.readable, 1, &made);
            pending->renewing = renew_the_pictures_a_copy_writes(one, bytes, copy, right, bottom,
                                                                 pending->renewed);
            pending->in_bytes = 1;
        } else {
            vkCmdCopyImageToBuffer(one, surface.image, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                                   surface.readable, 1, &out);
            if (copy->as_one_float) {
                pending->renewing = renew_the_floats_a_copy_writes(one, copy, &out, right, bottom,
                                                                   pending->renewed);
            }
        }

        /* A copy that may be the frame is also kept on the device, made smaller
         * there where each pixel was drawn as more than one sample, so that
         * showing it never has to come back through a host. */
        if (copy->left == 0 && copy->top == 0 && across <= XENOLITH_ACROSS
            && down <= XENOLITH_DOWN) {
            kept = a_copy_to_keep(copy->into, across, down);
        }
        if (kept < XENOLITH_COPIES_KEPT) {
            a_picture_becomes(one, copies[kept].image, VK_IMAGE_LAYOUT_UNDEFINED,
                              VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL);
            VkImageBlit smaller = {0};
            smaller.srcSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
            smaller.srcSubresource.layerCount = 1;
            smaller.srcOffsets[1].x = (int32_t)grid_across;
            smaller.srcOffsets[1].y = (int32_t)grid_down;
            smaller.srcOffsets[1].z = 1;
            smaller.dstSubresource = smaller.srcSubresource;
            smaller.dstOffsets[1].x = (int32_t)across;
            smaller.dstOffsets[1].y = (int32_t)down;
            smaller.dstOffsets[1].z = 1;
            vkCmdBlitImage(one, surface.image, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                           copies[kept].image, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1, &smaller,
                           VK_FILTER_LINEAR);
            a_picture_becomes(one, copies[kept].image, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                              VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL);
            copies[kept].where = copy->into;
            copies[kept].when = ++copies_made;
        }
    }

    for (uint32_t at = 0; at < pending->renewing; at++) {
        uint32_t which = pending->renewed[at];
        turned[which].renewed_by = copies_counted;
        turned[which].renewed_in = batches_begun;
        turned[which].stale = 0;
    }
    copies_waiting[lot]++;
    anything_recorded = 1;
    handed_back_at[lot] = lined_up(handed_back_at[lot] + room, 256u);
    pthread_mutex_unlock(&one_drawing);
    return 1;
}

/* One copy being put back where its title asked for it, as the threads that
 * share the work see it. */
struct a_copy_put_back {
    uint8_t *base;
    const struct xenolith_copy *copy;
    const unsigned char *from;
    uint32_t right;
    uint32_t squares_a_row;
    uint32_t across_by;
    uint32_t down_by;
    uint32_t grid_across;
    uint32_t samples;
    size_t sample_bytes;
    int in_bytes;
};

/* Puts back the rows of a copy from the first up to the last. */
static void put_rows_back(const struct a_copy_put_back *putting, uint32_t first_row,
                          uint32_t last_row) {
    uint8_t *base = putting->base;
    const struct xenolith_copy *copy = putting->copy;
    const unsigned char *from = putting->from;
    uint32_t right = putting->right;
    uint32_t squares_a_row = putting->squares_a_row;
    uint32_t across_by = putting->across_by;
    uint32_t down_by = putting->down_by;
    uint32_t grid_across = putting->grid_across;
    uint32_t samples = putting->samples;
    size_t sample_bytes = putting->sample_bytes;
    if (putting->in_bytes) {
        put_a_copy_in_squares(base, copy, from, right, squares_a_row, first_row, last_row);
        return;
    }
    for (uint32_t row = first_row; row < last_row; row++) {
        uint32_t down_to = (row / XENOLITH_A_SQUARE_ACROSS) * squares_a_row
                           * XENOLITH_WHAT_A_SQUARE_HOLDS;
        const uint32_t *in_the_square =
            &where_in_a_square[(row % XENOLITH_A_SQUARE_ACROSS) * XENOLITH_A_SQUARE_ACROSS];
        const unsigned char *first_row =
            from + (size_t)(row - copy->top) * down_by * grid_across * sample_bytes;

        for (uint32_t at = copy->left; at < right; at++) {
            const unsigned char *one_sample =
                first_row + (size_t)(at - copy->left) * across_by * sample_bytes;
            /* A place holding one float rather than four colours takes the
             * brightness on its own, which is what a title puts in such a
             * place: it works out how bright the whole scene came out by
             * halving a picture of it down to a single reading, and reads that
             * back to decide how bright to show the scene. Handed the four
             * colours packed instead, it reads their bytes as a float, which is
             * a number nothing sensible comes of, and the scene it then shows
             * is white. */
            uint32_t colour;
            if (copy->from_how_far) {
                /* Kept as a whole float on this side and as the console keeps
                 * it on the other: three bytes of it over one of the stencil a
                 * title tests against, which nothing here draws with. How far
                 * away is not averaged, because a place half way between two
                 * things is at neither. */
                float away;
                memcpy(&away, one_sample, sizeof away);
                uint32_t held;
                if (copy->how_far_as_a_float) {
                    held = a_short_float_from_how_far(away);
                } else {
                    float counted = away * 16777215.0f;
                    held = counted <= 0.0f ? 0u
                           : counted >= 16777215.0f ? 0xffffffu
                                                    : (uint32_t)counted;
                }
                colour = held << 8;
            } else {
                uint32_t part_of[4];
                float bright;
                if (samples == 1u) {
                    uint16_t halves[4];
                    memcpy(halves, one_sample, sizeof halves);
                    for (uint32_t part = 0; part < 4u; part++) {
                        part_of[part] = halves_as_bytes[halves[part]];
                    }
                    bright = halves_as_floats[halves[0]];
                } else {
                    float sum[4] = {0.0f, 0.0f, 0.0f, 0.0f};
                    for (uint32_t down_in = 0; down_in < down_by; down_in++) {
                        const unsigned char *sample =
                            one_sample + (size_t)down_in * grid_across * sample_bytes;
                        for (uint32_t across_in = 0; across_in < across_by; across_in++) {
                            uint16_t halves[4];
                            memcpy(halves, sample + across_in * sample_bytes, sizeof halves);
                            for (uint32_t part = 0; part < 4u; part++) {
                                sum[part] += halves_as_floats[halves[part]];
                            }
                        }
                    }
                    for (uint32_t part = 0; part < 4u; part++) {
                        part_of[part] = a_part_as_a_byte(sum[part] / (float)samples);
                    }
                    bright = sum[0] / (float)samples;
                }
                if (copy->as_one_float) {
                    memcpy(&colour, &bright, sizeof colour);
                } else if (copy->red_and_blue_swapped) {
                    colour = (part_of[3] << 24) | (part_of[0] << 16) | (part_of[1] << 8)
                             | part_of[2];
                } else {
                    colour = (part_of[3] << 24) | (part_of[2] << 16) | (part_of[1] << 8)
                             | part_of[0];
                }
            }
            uint32_t sits = down_to
                            + (at / XENOLITH_A_SQUARE_ACROSS) * XENOLITH_WHAT_A_SQUARE_HOLDS
                            + in_the_square[at % XENOLITH_A_SQUARE_ACROSS];
            xenolith_store32(base, copy->into + sits, colour);
        }
    }
}

/* Threads that share putting copies back between them, with the one that
 * hands over what was recorded. Each row of a copy lands in memory of its own,
 * so the rows can be put back in any order, and a frame of copies put back on
 * one thread took a third of the time the frame did. */
#define XENOLITH_PUTTERS_BACK 7u
#define XENOLITH_ROWS_AT_ONCE 16u

static struct {
    pthread_mutex_t lock;
    pthread_cond_t go;
    pthread_cond_t done;
    const struct a_copy_put_back *putting;
    uint32_t next_row;
    uint32_t last_row;
    unsigned long round;
    uint32_t busy;
    int started;
} sharing = {PTHREAD_MUTEX_INITIALIZER, PTHREAD_COND_INITIALIZER, PTHREAD_COND_INITIALIZER,
             NULL, 0, 0, 0, 0, 0};

/* Takes rows while any are left, with the lock held on the way in and out. */
static void take_rows_while_there_are_some(void) {
    while (sharing.next_row < sharing.last_row) {
        uint32_t first = sharing.next_row;
        uint32_t last = first + XENOLITH_ROWS_AT_ONCE;
        if (last > sharing.last_row) {
            last = sharing.last_row;
        }
        sharing.next_row = last;
        const struct a_copy_put_back *putting = sharing.putting;
        pthread_mutex_unlock(&sharing.lock);
        put_rows_back(putting, first, last);
        pthread_mutex_lock(&sharing.lock);
    }
}

static void *put_back_alongside(void *unused) {
    (void)unused;
    unsigned long seen = 0;
    pthread_mutex_lock(&sharing.lock);
    for (;;) {
        while (sharing.round == seen) {
            pthread_cond_wait(&sharing.go, &sharing.lock);
        }
        seen = sharing.round;
        sharing.busy++;
        take_rows_while_there_are_some();
        sharing.busy--;
        if (sharing.busy == 0) {
            pthread_cond_signal(&sharing.done);
        }
    }
    return NULL;
}

/* Puts a copy back, sharing its rows out where there are enough of them. */
static void put_back_in_parallel(const struct a_copy_put_back *putting, uint32_t first_row,
                                 uint32_t last_row) {
    if (last_row - first_row < XENOLITH_ROWS_AT_ONCE * 2u) {
        put_rows_back(putting, first_row, last_row);
        return;
    }
    pthread_mutex_lock(&sharing.lock);
    if (!sharing.started) {
        sharing.started = 1;
        for (uint32_t one = 0; one < XENOLITH_PUTTERS_BACK; one++) {
            pthread_t helper;
            if (pthread_create(&helper, NULL, put_back_alongside, NULL) == 0) {
                pthread_detach(helper);
            }
        }
    }
    sharing.putting = putting;
    sharing.next_row = first_row;
    sharing.last_row = last_row;
    sharing.round++;
    pthread_cond_broadcast(&sharing.go);
    take_rows_while_there_are_some();
    while (sharing.busy > 0) {
        pthread_cond_wait(&sharing.done, &sharing.lock);
    }
    pthread_mutex_unlock(&sharing.lock);
}

/* Puts what each waiting copy read back where its title asked for it, once
 * the lot the copies were recorded in has run.
 *
 * The colours go out the way round the hardware keeps them, red last, unless
 * the copy asks for red and blue to be swapped, which a title asks for where it
 * means to show what it copies.
 *
 * It goes out in squares rather than row by row, because that is the shape a
 * title reads it back in. The console copies into a picture, and a picture is
 * kept in squares, so a title that copies a scene out and then draws with it
 * hands the same squared reading to the hardware either way. Written row by
 * row it came back through that reading scrambled, which is how a scene drawn
 * right ended up as a checkerboard behind everything else. */
static void put_back_what_was_copied(uint32_t lot) {
    handed_back_at[lot] = (VkDeviceSize)lot * XENOLITH_HANDED_BACK_A_LOT;
    if (copies_waiting[lot] == 0) {
        return;
    }
    struct a_waiting_copy *pending = waiting[lot];
    pthread_mutex_lock(&one_reading);
    void *mapped = NULL;
    if (vkMapMemory(drawing.device, surface.readable_memory, 0, VK_WHOLE_SIZE, 0, &mapped)
            != VK_SUCCESS
        || mapped == NULL) {
        pthread_mutex_unlock(&one_reading);
        copies_waiting[lot] = 0;
        return;
    }
    const unsigned char *pixels = mapped;

    work_out_a_square();
    work_out_the_halves();

    for (uint32_t which = 0; which < copies_waiting[lot]; which++) {
        uint8_t *base = pending[which].base;
        const struct xenolith_copy *copy = &pending[which].copy;
        uint32_t right = pending[which].right;
        uint32_t bottom = pending[which].bottom;
        uint32_t across_by = copy->samples_across != 0 ? copy->samples_across : 1u;
        uint32_t down_by = copy->samples_down != 0 ? copy->samples_down : 1u;
        uint32_t grid_across = (right - copy->left) * across_by;
        int in_bytes = pending[which].in_bytes;

        /* A square holds four bytes here, and the squares are counted across a row
         * rounded up to the two and thirty a square's place is worked out in. */
        uint32_t squares_a_row = ((copy->pitch + 31u) & ~31u) / XENOLITH_A_SQUARE_ACROSS;
        uint32_t samples = across_by * down_by;
        size_t sample_bytes = copy->from_how_far ? 4u : XENOLITH_SURFACE_BYTES;

        struct a_copy_put_back putting = {
            .base = base,
            .copy = copy,
            .from = pixels + pending[which].at,
            .right = right,
            .squares_a_row = squares_a_row,
            .across_by = across_by,
            .down_by = down_by,
            .grid_across = grid_across,
            .samples = samples,
            .sample_bytes = sample_bytes,
            .in_bytes = in_bytes,
        };
        put_back_in_parallel(&putting, copy->top, bottom);
    }

    /* A picture put right on the device is no longer what its memory added up
     * to before, so that is worked out again, once copies stop going into it,
     * rather than its memory being taken for a title writing over it. */
    for (uint32_t which = 0; which < copies_waiting[lot]; which++) {
        for (uint32_t at = 0; at < pending[which].renewing; at++) {
            turned[pending[which].renewed[at]].came_to_unknown = 1;
        }
    }

    vkUnmapMemory(drawing.device, surface.readable_memory);
    pthread_mutex_unlock(&one_reading);
    copies_waiting[lot] = 0;
}

/* Hands over what has been recorded into the lot being recorded, without
 * waiting for it to run, and goes on to the next lot.
 *
 * A frame being drawn when this is asked for is ended first and carries on in
 * the next lot, as though nothing had happened, so a draw part of the way
 * through being set up loses nothing. */
static int send_what_was_recorded(void) {
    if (!batch_open || !anything_recorded) {
        return 1;
    }
    int was_recording = recording;
    if (was_recording) {
        xenolith_end_a_frame_inside();
    }
    uint32_t lot = current_lot;
    batch_open = 0;
    anything_recorded = 0;

    VkCommandBuffer handing_over[2];
    uint32_t count = 0;
    int handed = 1;
    if (lots[lot].before_open) {
        handed = vkEndCommandBuffer(lots[lot].before) == VK_SUCCESS;
        handing_over[count++] = lots[lot].before;
        lots[lot].before_open = 0;
    }
    handed = vkEndCommandBuffer(lots[lot].main) == VK_SUCCESS && handed;
    handing_over[count++] = lots[lot].main;
    if (handed) {
        VkSubmitInfo handing = {0};
        handing.sType = VK_STRUCTURE_TYPE_SUBMIT_INFO;
        handing.commandBufferCount = count;
        handing.pCommandBuffers = handing_over;
        vkResetFences(drawing.device, 1, &lots[lot].done);
        pthread_mutex_lock(&one_queue);
        handed = !went_wrong(vkQueueSubmit(drawing.queue, 1, &handing, lots[lot].done),
                             "what was recorded");
        pthread_mutex_unlock(&one_queue);
    }
    lots[lot].in_flight = handed;
    lots[lot].sent_at = ++lots_sent;
    last_sent_lot = lot;
    current_lot = (lot + 1u) % XENOLITH_LOTS;

    if (was_recording) {
        return xenolith_begin_a_frame_inside() && handed;
    }
    return handed;
}

/* Waits for a lot handed over to have run, after any handed over before it,
 * and then puts back what its copies read, lets go of what it was the last to
 * read, and frees the room its pictures were handed over through. */
static void finish_the_lot(uint32_t lot) {
    for (uint32_t other = 0; other < XENOLITH_LOTS; other++) {
        if (other != lot && lots[other].in_flight && lots[other].sent_at < lots[lot].sent_at) {
            finish_the_lot(other);
        }
    }
    if (lots[lot].in_flight) {
        went_wrong(vkWaitForFences(drawing.device, 1, &lots[lot].done, VK_TRUE, UINT64_MAX),
                   "waiting for what was recorded");
        lots[lot].in_flight = 0;
    }
    put_back_what_was_copied(lot);
    let_go_of_what_is_finished(lot);
    for (uint32_t at = 0; at < lots[lot].holding; at++) {
        vkDestroyBuffer(drawing.device, lots[lot].held[at], NULL);
        vkFreeMemory(drawing.device, lots[lot].held_memory[at], NULL);
    }
    lots[lot].holding = 0;
}

/* Finishes every lot that has already run, oldest first, without waiting for
 * one that has not. */
static void finish_what_has_run(void) {
    for (;;) {
        uint32_t oldest = XENOLITH_LOTS;
        for (uint32_t lot = 0; lot < XENOLITH_LOTS; lot++) {
            if (lots[lot].in_flight
                && (oldest == XENOLITH_LOTS || lots[lot].sent_at < lots[oldest].sent_at)) {
                oldest = lot;
            }
        }
        if (oldest == XENOLITH_LOTS
            || vkGetFenceStatus(drawing.device, lots[oldest].done) != VK_SUCCESS) {
            return;
        }
        finish_the_lot(oldest);
    }
}

/* Hands over everything recorded so far, waits for all of it to run, and puts
 * what its copies read back in place. */
static int hand_over_what_was_recorded(void) {
    int handed = send_what_was_recorded();
    for (uint32_t lot = 0; lot < XENOLITH_LOTS; lot++) {
        if (!(batch_open && lot == current_lot)) {
            finish_the_lot(lot);
        }
    }
    if (!batch_open) {
        /* Nothing recorded reads the room, so all of it can be given out. */
        give_out_the_room_again();
    }
    return handed;
}

/* Waits for whatever copies are still to write memory a title is about to
 * read a picture out of, and only for those. */
static void wait_for_copies_into(uint32_t from, uint32_t over) {
    pthread_mutex_lock(&one_drawing);
    for (uint32_t lot = 0; lot < XENOLITH_LOTS; lot++) {
        if (!a_copy_is_still_to_write(lot, from, over)) {
            continue;
        }
        if (batch_open && lot == current_lot) {
            send_what_was_recorded();
        }
        finish_the_lot(lot);
    }
    pthread_mutex_unlock(&one_drawing);
}

int xenolith_send_what_was_drawn(void) {
    pthread_mutex_lock(&one_drawing);
    int went = send_what_was_recorded();
    finish_what_has_run();
    pthread_mutex_unlock(&one_drawing);
    return went;
}

int xenolith_wait_for_what_was_drawn(void) {
    pthread_mutex_lock(&one_drawing);
    int went = hand_over_what_was_recorded();
    pthread_mutex_unlock(&one_drawing);
    return went;
}

int xenolith_keep_a_frame(const char *where) {
    if (!surface.made) {
        return 0;
    }
    xenolith_wait_for_what_was_drawn();

    /* Fetched first. Nothing else puts what is shown where a host can read it
     * any more, and this is only ever asked for by a run that says to write
     * frames out, so what it costs does not matter. A copy kept to be shown
     * holds bytes and the surface holds half floats. */
    VkImage shown = what_was_shown != VK_NULL_HANDLE ? what_was_shown : surface.image;
    int in_halves = shown == surface.image;
    {
        VkCommandBuffer one = begin_one_lot_of_commands();
        if (one != VK_NULL_HANDLE) {
            VkBufferImageCopy out = {0};
            out.imageSubresource.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
            out.imageSubresource.layerCount = 1;
            out.imageExtent.width = XENOLITH_ACROSS;
            out.imageExtent.height = XENOLITH_DOWN;
            out.imageExtent.depth = 1;
            vkCmdCopyImageToBuffer(one, shown, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                                   surface.readable, 1, &out);
            finish_one_lot_of_commands(one);
        }
    }

    pthread_mutex_lock(&one_reading);

    void *pixels = NULL;
    if (vkMapMemory(drawing.device, surface.readable_memory, 0, VK_WHOLE_SIZE, 0, &pixels)
            != VK_SUCCESS
        || pixels == NULL) {
        pthread_mutex_unlock(&one_reading);
        return 0;
    }

    FILE *file = fopen(where, "wb");
    if (file == NULL) {
        vkUnmapMemory(drawing.device, surface.readable_memory);
        pthread_mutex_unlock(&one_reading);
        return 0;
    }

    fprintf(file, "P6\n%u %u\n255\n", XENOLITH_ACROSS, XENOLITH_DOWN);
    const unsigned char *from = pixels;
    for (uint32_t at = 0; at < XENOLITH_ACROSS * XENOLITH_DOWN; at++) {
        if (!in_halves) {
            fwrite(from + (size_t)at * 4, 1, 3, file);
            continue;
        }
        for (uint32_t part = 0; part < 3u; part++) {
            fputc((int)a_part_as_a_byte(
                      a_half(from + (size_t)at * XENOLITH_SURFACE_BYTES + part * 2u)),
                  file);
        }
    }

    fclose(file);
    vkUnmapMemory(drawing.device, surface.readable_memory);
    pthread_mutex_unlock(&one_reading);
    return 1;
}

int xenolith_can_draw(void) { return open_something_to_draw_with(); }

#else

int xenolith_show_what_was_copied_to(uint32_t where) {
    (void)where;
    return 0;
}

int xenolith_show_the_frame(void) { return 0; }

void xenolith_forget_pictures_at(uint32_t where, uint32_t bytes) {
    (void)where;
    (void)bytes;
}

int xenolith_wait_for_what_was_drawn(void) { return 0; }

int xenolith_send_what_was_drawn(void) { return 0; }

uint32_t *xenolith_room_for_vertices(uint32_t *how_many) {
    (void)how_many;
    return NULL;
}

uint32_t xenolith_where_the_room_begins(uint32_t *given_out) {
    if (given_out != NULL) {
        *given_out = 0;
    }
    return 0;
}

void xenolith_streams_begin_at(const uint32_t *where) { (void)where; }

void xenolith_done_with_vertices(uint32_t words) { (void)words; }

void xenolith_number_the_vertices(uint32_t at, uint32_t count, uint32_t first) {
    (void)at;
    (void)count;
    (void)first;
}

void xenolith_push_away_by(float outright, float along_a_slope) {
    (void)outright;
    (void)along_a_slope;
}

void xenolith_move_a_position_by(const float *scale, const float *offset) {
    (void)scale;
    (void)offset;
}


void xenolith_clip_to(uint32_t left, uint32_t top, uint32_t right, uint32_t bottom) {
    (void)left;
    (void)top;
    (void)right;
    (void)bottom;
}

void xenolith_look_at(const float *scale, const float *offset) {
    (void)scale;
    (void)offset;
}

int xenolith_hand_over_constants(const uint8_t *from, const uint8_t *truths,
                                 const uint8_t *counts) {
    (void)from;
    (void)truths;
    (void)counts;
    return 0;
}

int xenolith_begin_a_frame(void) { return 0; }

int xenolith_draw_now(uint32_t blending, uint32_t written, uint32_t how_far, uint32_t cut_off,
                      uint32_t cut_off_at, uint32_t vertex_sum, const uint32_t *vertex,
                      uint32_t vertex_length, uint32_t pixel_sum, const uint32_t *pixel,
                      uint32_t pixel_length, uint32_t vertices, uint32_t kind) {
    (void)blending;
    (void)written;
    (void)how_far;
    (void)cut_off;
    (void)cut_off_at;
    (void)kind;
    (void)vertex_sum;
    (void)vertex;
    (void)vertex_length;
    (void)pixel_sum;
    (void)pixel;
    (void)pixel_length;
    (void)vertices;
    return 0;
}

int xenolith_end_a_frame(void) { return 0; }

int xenolith_can_draw_with(uint32_t vertex_sum, const uint32_t *vertex, uint32_t vertex_length,
                           uint32_t pixel_sum, const uint32_t *pixel, uint32_t pixel_length) {
    (void)vertex_sum;
    (void)vertex;
    (void)vertex_length;
    (void)pixel_sum;
    (void)pixel;
    (void)pixel_length;
    return 0;
}

int xenolith_hand_back_what_was_drawn(uint8_t *base, const struct xenolith_copy *copy) {
    (void)base;
    (void)copy;
    return 0;
}

int xenolith_keep_a_frame(const char *where) {
    (void)where;
    return 0;
}

int xenolith_what_is_held(uint16_t *buttons, int16_t *thumbs) {
    (void)buttons;
    (void)thumbs;
    return 0;
}

int xenolith_look_at_a_picture(uint32_t slot, const uint8_t *from, uint32_t where, uint32_t across,
                               uint32_t down, uint32_t pitch, int in_squares, uint32_t order,
                               uint32_t kept, uint32_t held_how) {
    (void)slot;
    (void)from;
    (void)where;
    (void)across;
    (void)down;
    (void)pitch;
    (void)in_squares;
    (void)order;
    (void)kept;
    (void)held_how;
    return 0;
}

void xenolith_look_at_nothing(uint32_t slot) { (void)slot; }

void xenolith_drawing_into(uint32_t shaped, uint32_t across, uint32_t down) {
    (void)shaped;
    (void)across;
    (void)down;
}

void xenolith_find_the_pixel_in(uint32_t which) { (void)which; }

void xenolith_wipe_how_far_after_a_copy(int wipe, float to) {
    (void)wipe;
    (void)to;
}

void xenolith_wipe_the_colours_after_a_copy(int wipe, uint32_t to) {
    (void)wipe;
    (void)to;
}

int xenolith_can_draw(void) {
    static int said;
    if (!said) {
        said = 1;
        fprintf(stderr, "xenolith: nothing here draws, because this was built without one\n");
    }
    return 0;
}

#endif /* XENOLITH_WITH_VULKAN */
